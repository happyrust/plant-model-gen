//! rkyv payload wrapper for foyer cache entries.
//!
//! 约定：
//! - foyer 的 Key/Value 仍按既有方式编码（由 foyer/hybridcache 管理）。
//! - Value 中的 `payload: Vec<u8>` 由本模块统一写入/读取。
//! - 迁移策略（方案1）：读到非 rkyv payload 或 schema/type 不匹配，一律视为 miss。

use anyhow::anyhow;
use std::hash::Hasher;
use twox_hash::XxHash64;

pub const MAGIC: [u8; 4] = *b"AIOS";
pub const CODEC: [u8; 4] = *b"RKYV";

/// header = magic(4) + codec(4) + type_tag(u16 LE) + schema_version(u16 LE)
///        + body_len(u32 LE) + body_xxhash64(u64 LE)
pub const HEADER_LEN: usize = 4 + 4 + 2 + 2 + 4 + 8;

#[inline]
fn xxhash64(bytes: &[u8]) -> u64 {
    let mut h = XxHash64::default();
    h.write(bytes);
    h.finish()
}

#[inline]
pub fn encode<T>(type_tag: u16, schema_version: u16, value: &T) -> anyhow::Result<Vec<u8>>
where
    T: rkyv::Archive,
    for<'a> T: rkyv::Serialize<
        rkyv::api::high::HighSerializer<rkyv::util::AlignedVec, rkyv::ser::allocator::ArenaHandle<'a>, rkyv::rancor::Error>,
    >,
{
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(value)
        .map_err(|e| anyhow!("rkyv 序列化失败: {:?}", e))?;
    let body_len: u32 = bytes
        .len()
        .try_into()
        .map_err(|_| anyhow!("payload 过大: len={}", bytes.len()))?;
    let body_hash = xxhash64(&bytes);
    let mut out = Vec::with_capacity(HEADER_LEN + bytes.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&CODEC);
    out.extend_from_slice(&type_tag.to_le_bytes());
    out.extend_from_slice(&schema_version.to_le_bytes());
    out.extend_from_slice(&body_len.to_le_bytes());
    out.extend_from_slice(&body_hash.to_le_bytes());
    out.extend_from_slice(&bytes);
    Ok(out)
}

#[inline]
pub fn decode<T>(type_tag: u16, schema_version: u16, payload: &[u8]) -> anyhow::Result<T>
where
    T: rkyv::Archive,
    <T as rkyv::Archive>::Archived: rkyv::Deserialize<
        T,
        rkyv::rancor::Strategy<rkyv::de::Pool, rkyv::rancor::Error>,
    >,
{
    if payload.len() < HEADER_LEN {
        return Err(anyhow!("payload 太短"));
    }
    if payload[0..4] != MAGIC {
        return Err(anyhow!("magic 不匹配"));
    }
    if payload[4..8] != CODEC {
        return Err(anyhow!("codec 不匹配"));
    }
    let got_type = u16::from_le_bytes([payload[8], payload[9]]);
    let got_schema = u16::from_le_bytes([payload[10], payload[11]]);
    let got_body_len = u32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]) as usize;
    let got_body_hash = u64::from_le_bytes([
        payload[16], payload[17], payload[18], payload[19], payload[20], payload[21], payload[22], payload[23],
    ]);
    if got_type != type_tag {
        return Err(anyhow!(
            "type_tag 不匹配: got={}, expected={}",
            got_type,
            type_tag
        ));
    }
    if got_schema != schema_version {
        return Err(anyhow!(
            "schema_version 不匹配: got={}, expected={}",
            got_schema,
            schema_version
        ));
    }
    let body = &payload[HEADER_LEN..];
    if body.len() != got_body_len {
        return Err(anyhow!(
            "body_len 不匹配: got={}, actual={}",
            got_body_len,
            body.len()
        ));
    }
    if xxhash64(body) != got_body_hash {
        return Err(anyhow!("body hash 不匹配（疑似损坏）"));
    }

    // body 来自普通 Vec<u8> 的子切片（偏移 HEADER_LEN），不保证满足 rkyv archived 类型的对齐要求。
    // nightly Rust (1.93+) 对 slice::from_raw_parts 的对齐前置条件做了严格检查，
    // 直接传入未对齐的 &[u8] 会触发 UB panic。
    // 解决方案：拷贝到 AlignedVec 再反序列化，保证 16 字节对齐。
    let mut aligned: rkyv::util::AlignedVec<16> = rkyv::util::AlignedVec::with_capacity(body.len());
    aligned.extend_from_slice(body);

    // SAFETY: aligned 内容与 body 完全一致（已校验 hash），且满足对齐要求。
    // 不用 checked(from_bytes) 是因为部分依赖类型（如 glam::Vec3）缺少 CheckBytes 实现。
    unsafe { rkyv::from_bytes_unchecked::<T, rkyv::rancor::Error>(&aligned) }
        .map_err(|e| anyhow!("rkyv 反序列化失败: {:?}", e))
}

#[inline]
pub fn is_rkyv_payload(payload: &[u8]) -> bool {
    payload.len() >= 8 && payload[0..4] == MAGIC && payload[4..8] == CODEC
}
