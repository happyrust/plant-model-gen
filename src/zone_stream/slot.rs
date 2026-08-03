//! ZONE 工作区的双缓冲主管（ADR-0016 D3 / D4）。
//!
//! 两个常驻 supervisor 各管一个 slot。它们要解决的核心问题不是调度，而是**身份**：
//! slot 会被反复清空复用，如果一个晚到的读请求拿着上一轮的 slot 引用去查当前 slot，
//! 它会静默读到另一个 ZONE 的数据 —— 双缓冲下这是正确性问题，不是健壮性优化。
//!
//! 因此每次 slot 被重新占用都会 bump 一次 generation，并发一张新的 [`SlotLease`]；
//! 复合读 session 的 snapshot 身份里带着 lease，过期 lease 一律硬失败（D4）。

use anyhow::{bail, Result};

use super::sidecar::{SLOT_A_DB, SLOT_B_DB};
use super::run_store::SlotState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotId {
    A,
    B,
}

impl SlotId {
    /// sidecar 里对应的逻辑数据库名。
    pub fn database(self) -> &'static str {
        match self {
            Self::A => SLOT_A_DB,
            Self::B => SLOT_B_DB,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "slot-a",
            Self::B => "slot-b",
        }
    }

    /// 双缓冲交替：当前 ZONE 在一个 slot 下游推进时，下一个 ZONE 解析进另一个。
    pub fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

/// 一次 slot 占用的凭证。
///
/// `generation` 单调递增且永不复用，这是防 ABA 的关键：slot 清空再占用后
/// 旧 lease 的 generation 必然落后，拿旧 lease 读会被 [`SlotSupervisor::validate`] 拒掉。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotLease {
    pub slot: SlotId,
    pub generation: u64,
}

pub struct SlotSupervisor {
    slot: SlotId,
    generation: u64,
    state: SlotState,
    /// 当前占用该 slot 的 ZONE refno；`None` 表示空闲。
    occupant: Option<String>,
}

impl SlotSupervisor {
    pub fn new(slot: SlotId) -> Self {
        Self {
            slot,
            generation: 0,
            state: SlotState::Empty,
            occupant: None,
        }
    }

    pub fn slot(&self) -> SlotId {
        self.slot
    }

    pub fn state(&self) -> SlotState {
        self.state
    }

    pub fn occupant(&self) -> Option<&str> {
        self.occupant.as_deref()
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.state, SlotState::Empty)
    }

    /// 占用空闲 slot 承载一个 ZONE，bump generation 并发新 lease。
    pub fn acquire(&mut self, zone_refno: &str) -> Result<SlotLease> {
        if !self.is_idle() {
            bail!(
                "{} 仍被 ZONE `{}` 占用（状态 {}），不能装载 ZONE `{}`",
                self.slot.as_str(),
                self.occupant.as_deref().unwrap_or("<unknown>"),
                self.state.as_str(),
                zone_refno
            );
        }
        self.generation += 1;
        self.state = SlotState::Parsing;
        self.occupant = Some(zone_refno.to_string());
        Ok(SlotLease {
            slot: self.slot,
            generation: self.generation,
        })
    }

    /// 校验 lease 是否仍指向当前这一轮占用。过期即硬失败，不做任何回退查询。
    pub fn validate(&self, lease: SlotLease) -> Result<()> {
        if lease.slot != self.slot {
            bail!(
                "lease 指向 {}，但由 {} 校验",
                lease.slot.as_str(),
                self.slot.as_str()
            );
        }
        if lease.generation != self.generation {
            bail!(
                "{} 的 lease 已过期（lease generation={}，当前 generation={}）：\
                 该 slot 已被清空复用，继续读会串到另一个 ZONE 的数据",
                self.slot.as_str(),
                lease.generation,
                self.generation
            );
        }
        Ok(())
    }

    /// 推进阶段。只允许沿 Parsing → Sealed → Generating → Backfilling 单向前进，
    /// 倒退或跳跃说明编排出错，直接失败而不是容忍。
    pub fn advance(&mut self, lease: SlotLease, next: SlotState) -> Result<()> {
        self.validate(lease)?;
        let ok = matches!(
            (self.state, next),
            (SlotState::Parsing, SlotState::Sealed)
                | (SlotState::Sealed, SlotState::Generating)
                | (SlotState::Generating, SlotState::Backfilling)
        );
        if !ok {
            bail!(
                "{} 不能从 {} 直接进入 {}",
                self.slot.as_str(),
                self.state.as_str(),
                next.as_str()
            );
        }
        self.state = next;
        Ok(())
    }

    /// ZONE 回填完成（或整轮放弃）后清空 slot。
    ///
    /// 不 bump generation —— 下一次 [`Self::acquire`] 才 bump。这样「已释放但尚未被
    /// 重新占用」期间，旧 lease 依然能被 [`Self::validate`] 识别为当前 generation，
    /// 由 [`Self::state`] 为 `Empty` 表达「这一轮已经结束」。
    pub fn release(&mut self) {
        self.state = SlotState::Empty;
        self.occupant = None;
    }
}

/// 双 slot 的持有者。ADR-0016 D2 要求同一时刻最多一个 generator、一个 backfill，
/// 所以下游阶段的互斥在这里表达为「至多一个 slot 处于 Generating/Backfilling」。
pub struct SlotPair {
    pub a: SlotSupervisor,
    pub b: SlotSupervisor,
}

impl Default for SlotPair {
    fn default() -> Self {
        Self::new()
    }
}

impl SlotPair {
    pub fn new() -> Self {
        Self {
            a: SlotSupervisor::new(SlotId::A),
            b: SlotSupervisor::new(SlotId::B),
        }
    }

    pub fn get(&self, slot: SlotId) -> &SlotSupervisor {
        match slot {
            SlotId::A => &self.a,
            SlotId::B => &self.b,
        }
    }

    pub fn get_mut(&mut self, slot: SlotId) -> &mut SlotSupervisor {
        match slot {
            SlotId::A => &mut self.a,
            SlotId::B => &mut self.b,
        }
    }

    /// 是否已有 slot 处于下游阶段（生成或回填）。
    ///
    /// ADR-0016 D2：允许「解析下一 ZONE」与「生成/回填当前 ZONE」重叠，
    /// 但不允许两个 generator 或两个 backfill 并行，所以下一个 ZONE 想进入下游前必须先问这个。
    pub fn downstream_busy(&self) -> bool {
        [&self.a, &self.b].iter().any(|supervisor| {
            matches!(
                supervisor.state(),
                SlotState::Generating | SlotState::Backfilling
            )
        })
    }

    /// 挑一个空闲 slot 去解析下一个 ZONE。
    pub fn idle_slot(&self) -> Option<SlotId> {
        if self.a.is_idle() {
            Some(SlotId::A)
        } else if self.b.is_idle() {
            Some(SlotId::B)
        } else {
            None
        }
    }
}
