# -*- coding: utf-8 -*-
"""specs/022 图2:sesno 锚点固化与历史查询流程图 生成器。

重新生成:
    python gen_02_anchor_and_query_flow.py
输出 02-anchor-and-query-flow.svg 到本脚本所在目录。
"""
import xml.etree.ElementTree as ET
from pathlib import Path

W, H = 960, 760
SANS = "Microsoft YaHei, PingFang SC, Segoe UI, Arial, sans-serif"
MONO = "Consolas, Menlo, monospace"

MARKERS = {
    "#2563eb": "arrow-blue",
    "#059669": "arrow-green",
    "#dc2626": "arrow-red",
    "#6b7280": "arrow-gray",
}

L = []
A = L.append


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def text(x, y, s, size=10.5, fill="#334155", anchor="middle", bold=False, mono=False):
    fam = MONO if mono else SANS
    w = ' font-weight="bold"' if bold else ""
    A(f'  <text x="{x}" y="{y}" font-size="{size}" fill="{fill}" text-anchor="{anchor}" font-family="{fam}"{w}>{esc(s)}</text>')


def box(x, y, w, h, fill, stroke):
    A(f'  <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="8" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>')


def container(x, y, w, h):
    A(f'  <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="10" fill="none" stroke="#94a3b8" stroke-width="1.2" stroke-dasharray="5,4"/>')


def arrow(d, color, dash=None, sw=1.8):
    dd = f' stroke-dasharray="{dash}"' if dash else ""
    A(f'  <path d="{d}" fill="none" stroke="{color}" stroke-width="{sw}"{dd} marker-end="url(#{MARKERS[color]})"/>')


def label_bg(x, y, w, h):
    A(f'  <rect x="{x}" y="{y}" width="{w}" height="{h}" fill="#ffffff" opacity="0.95"/>')


def cylinder(x0, y0, x1, y1, fill, stroke, ry=10):
    cx = (x0 + x1) / 2
    rx = (x1 - x0) / 2
    A(f'  <ellipse cx="{cx}" cy="{y1 - ry}" rx="{rx}" ry="{ry}" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>')
    A(f'  <rect x="{x0}" y="{y0 + ry}" width="{x1 - x0}" height="{y1 - y0 - 2 * ry}" fill="{fill}"/>')
    A(f'  <line x1="{x0}" y1="{y0 + ry}" x2="{x0}" y2="{y1 - ry}" stroke="{stroke}" stroke-width="1.5"/>')
    A(f'  <line x1="{x1}" y1="{y0 + ry}" x2="{x1}" y2="{y1 - ry}" stroke="{stroke}" stroke-width="1.5"/>')
    A(f'  <ellipse cx="{cx}" cy="{y0 + ry}" rx="{rx}" ry="{ry}" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>')


# ---------------- document ----------------
A(f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">')
A("  <defs>")
for color, mid in MARKERS.items():
    A(f'    <marker id="{mid}" markerWidth="10" markerHeight="7" refX="9" refY="3.5" orient="auto" markerUnits="userSpaceOnUse">')
    A(f'      <path d="M0,0 L10,3.5 L0,7 z" fill="{color}"/>')
    A("    </marker>")
A("  </defs>")
A(f'  <rect x="0" y="0" width="{W}" height="{H}" fill="#ffffff"/>')

# 标题
text(40, 38, "022 PE/ATT 版本化存储 · 图 2:sesno 锚点固化与历史查询", 18, "#0f172a", "start", bold=True)
text(40, 58, "段 A:增量落库全部成功后才固化 sesno→时间戳锚点;段 B:业务按 sesno 经锚点换算 $t 走 VERSION 时间旅行查询 (spec 022 · US1/US2)", 11.5, "#64748b", "start")

# ---------------- 容器 ----------------
container(24, 70, 912, 226)   # 段 A
container(24, 308, 912, 150)  # 存储层
container(24, 468, 912, 240)  # 段 B
text(40, 92, "段 A · 增量落库自动固化锚点(US2 · FR-003 / FR-004)", 13, "#334155", "start", bold=True)
text(40, 330, "存储层 · SUL_DB(RocksDB versioned)", 12.5, "#334155", "start", bold=True)
text(40, 452, "rocksdb://…?versioned=true&retention=<r> · 默认0=无限保留,可配90d/30d", 9, "#64748b", "start")
text(40, 490, "段 B · 按 sesno 历史查询(US1 · FR-005 / FR-007)", 13, "#334155", "start", bold=True)

# ---------------- 箭头(先画,盒子后画压住端点缝隙) ----------------
# 段 A 主流程
arrow("M 220,154 L 248,154", "#2563eb")
arrow("M 432,154 L 452,154", "#2563eb")
arrow("M 648,154 L 660,154", "#2563eb")
arrow("M 760,154 L 778,154", "#2563eb")           # 判定→落锚(成功)
text(769, 147, "成功", 9.5, "#16a34a", bold=True)
arrow("M 712,192 L 712,212", "#dc2626")            # 判定→不写锚点(失败)
text(718, 207, "失败", 9.5, "#dc2626", "start", bold=True)

# A3 → PE/ATT versioned 存储(版本化写入)
arrow("M 552,196 L 552,282 L 806,282 L 810,282 L 810,338", "#059669", dash="5,3", sw=1.6)
label_bg(617, 268, 126, 15)
text(680, 279, "版本化写入(HLC 时间戳)", 9.5, "#047857")

# A5 → 锚点表(成功后落锚),在 x=552 处跳线避让 A3 的下行线
arrow("M 859,196 L 859,270 L 560,270 A 8,8 0 0 0 544,270 L 400,270 L 400,338", "#059669", dash="5,3", sw=1.6)
label_bg(452, 254, 62, 15)
text(483, 265, "固化锚点", 9.5, "#047857")

# 全量首锚旁注 → 锚点表
arrow("M 252,386 L 276,386", "#6b7280", dash="4,3", sw=1.4)

# 存储层 → 段 B 读取
arrow("M 400,426 L 400,503", "#059669", sw=1.6)    # 锚点表 → resolve_anchor
label_bg(406, 470, 152, 15)
text(412, 481, "sesno → anchored_at($t)", 9.5, "#047857", "start")
arrow("M 810,426 L 810,446 L 603,446 L 603,503", "#059669", sw=1.6)  # versioned 存储 → VERSION 查询
label_bg(636, 432, 120, 15)
text(696, 443, "VERSION $t 历史读取", 9.5, "#047857")

# 段 B 主流程
arrow("M 226,543 L 258,543", "#2563eb")
arrow("M 456,543 L 492,543", "#2563eb")
label_bg(458, 522, 36, 14)
text(476, 533, "携 $t", 9.5, "#1d4ed8")
arrow("M 710,543 L 760,543", "#2563eb")
arrow("M 359,579 L 359,626", "#dc2626")            # 锚点缺失
text(365, 606, "缺失", 9, "#dc2626", "start", bold=True)
arrow("M 603,579 L 603,620", "#dc2626")            # GC 越界
text(609, 602, "$t 越界", 9, "#dc2626", "start", bold=True)

# ---------------- 段 A 节点 ----------------
box(40, 112, 180, 84, "#e0f2fe", "#0284c7")
text(130, 140, "watch-incremental 队列", 12, "#0c4a6e", bold=True)
text(130, 160, "同一 dbnum 串行执行", 10.5, "#475569")
text(130, 176, "(锚点一致性前提)", 9.5, "#64748b")

box(252, 112, 180, 84, "#e0f2fe", "#0284c7")
text(342, 138, "persist_pdms_increment", 11.5, "#0c4a6e", bold=True, mono=True)
text(342, 158, "增量落库 sesno:N → M", 10.5, "#475569")
text(342, 174, "全部 PE/ATT 变更成批写入", 9.5, "#64748b")

box(456, 112, 192, 84, "#e0f2fe", "#0284c7")
text(552, 134, "PE/ATT 批量", 12, "#0c4a6e", bold=True)
text(552, 150, "UPSERT / DELETE", 11.5, "#0c4a6e", bold=True, mono=True)
text(552, 167, "每次写入引擎自动打 HLC 时间戳", 9.5, "#475569")
text(552, 181, "旧版本保留(MVCC)", 9.5, "#64748b")

A('  <polygon points="664,154 712,116 760,154 712,192" fill="#fef3c7" stroke="#d97706" stroke-width="1.5"/>')
text(712, 158, "全部成功?", 12, "#78350f", bold=True)

box(782, 112, 154, 84, "#dcfce7", "#16a34a")
text(859, 132, "UPSERT 锚点", 12, "#14532d", bold=True)
text(859, 149, "{dbnum, sesno: M,", 9, "#166534", mono=True)
text(859, 163, "anchored_at: time::now(),", 9, "#166534", mono=True)
text(859, 177, 'source: "incremental"}', 9, "#166534", mono=True)

box(612, 216, 200, 42, "#fee2e2", "#dc2626")
text(712, 233, "不写锚点(FR-004)", 11.5, "#7f1d1d", bold=True)
text(712, 250, "无锚点的中间时间戳不对业务暴露", 9.5, "#991b1b")

# ---------------- 存储层 ----------------
# 全量首锚旁注
A('  <rect x="48" y="344" width="204" height="84" rx="8" fill="#f8fafc" stroke="#94a3b8" stroke-width="1.2" stroke-dasharray="4,3"/>')
text(150, 364, "另注 · 全量重灌首条锚点", 11, "#475569", bold=True)
text(150, 382, "sync_pdms 全量重灌完成后", 9.5, "#64748b")
text(150, 397, 'UPSERT source:"full" 锚点', 9.5, "#64748b", mono=True)
text(150, 412, "(该 dbnum 当前 latest_sesno)", 9, "#94a3b8")

cylinder(280, 340, 520, 424, "#fef9c3", "#ca8a04")
text(400, 376, "sesno_version_anchor", 12.5, "#713f12", bold=True, mono=True)
text(400, 393, "{dbnum, sesno → anchored_at, source}", 9.5, "#854d0e", mono=True)
text(400, 408, "业务历史查询的唯一入口(无锚不可见)", 9.5, "#854d0e")

cylinder(700, 340, 920, 424, "#dbeafe", "#2563eb")
text(810, 376, "PE / noun 表 / ATT_UDA", 12, "#1e3a8a", bold=True)
text(810, 393, "MVCC 全历史 · HLC 时间戳", 9.5, "#1e40af")
text(810, 408, "硬 DELETE 前记录仍可回溯", 9.5, "#1e40af")

# ---------------- 段 B 节点 ----------------
box(40, 507, 186, 72, "#e0f2fe", "#0284c7")
text(133, 528, "CLI · model-version", 12, "#0c4a6e", bold=True)
text(133, 546, "history snapshot", 11.5, "#0c4a6e", bold=True, mono=True)
text(133, 565, "--refno E --sesno N", 9.5, "#475569", mono=True)

box(262, 507, 194, 72, "#ede9fe", "#7c3aed")
text(359, 524, "rs-core · version_query", 11.5, "#4c1d95", bold=True)
text(359, 540, "resolve_anchor(dbnum, N)", 10, "#5b21b6", mono=True)
text(359, 556, "精确命中 / 「最近不大于」回退", 9.5, "#6d28d9")
text(359, 571, "(回退时标注 exact:false)", 9, "#7c3aed")

box(496, 507, 214, 72, "#e0f2fe", "#0284c7")
text(603, 524, "VERSION 时间旅行查询", 12, "#0c4a6e", bold=True)
text(603, 541, "SELECT * FROM pe:<refno>", 9.5, "#475569", mono=True)
text(603, 555, "VERSION $t", 9.5, "#475569", mono=True)
text(603, 571, "+ ATT 表同刻(同一 $t)查询", 9.5, "#64748b")

box(764, 507, 172, 72, "#dcfce7", "#16a34a")
text(850, 526, "组装历史快照返回", 12, "#14532d", bold=True)
text(850, 543, "PE + ATT 同刻状态", 9.5, "#166534")
text(850, 557, "含硬 DELETE 前的记录", 9.5, "#166534")
text(850, 571, "(SC-001:单元素 < 2s)", 9, "#16a34a")

box(262, 630, 194, 50, "#fee2e2", "#dc2626")
text(359, 650, "锚点完全缺失 → 报错", 11, "#7f1d1d", bold=True)
text(359, 667, "无任何 ≤ N 的锚点可回退(FR-005)", 9, "#991b1b")

box(470, 624, 286, 62, "#fee2e2", "#dc2626")
text(613, 641, "$t 低于 GC 水位线 full_history_ts_low", 9.5, "#991b1b")
text(613, 657, "InvalidArgument → HistoryExpired", 11, "#7f1d1d", bold=True)
text(613, 673, "「该 sesno 历史已超出 retention=90d 窗口」", 9.5, "#991b1b")

# ---------------- 图例 ----------------
text(40, 738, "图例", 11, "#334155", "start", bold=True)
A('  <path d="M 86,734 L 122,734" stroke="#2563eb" stroke-width="2" fill="none" marker-end="url(#arrow-blue)"/>')
text(128, 738, "主流程", 10, "#334155", "start")
A('  <path d="M 206,734 L 242,734" stroke="#059669" stroke-width="1.6" stroke-dasharray="5,3" fill="none" marker-end="url(#arrow-green)"/>')
text(248, 738, "版本化写入(引擎自动 HLC 时间戳)", 10, "#334155", "start")
A('  <path d="M 452,734 L 488,734" stroke="#059669" stroke-width="1.6" fill="none" marker-end="url(#arrow-green)"/>')
text(494, 738, "存储读取", 10, "#334155", "start")
A('  <path d="M 560,734 L 596,734" stroke="#dc2626" stroke-width="1.8" fill="none" marker-end="url(#arrow-red)"/>')
text(602, 738, "失败 / 异常路径", 10, "#334155", "start")
A('  <path d="M 714,734 L 750,734" stroke="#6b7280" stroke-width="1.4" stroke-dasharray="4,3" fill="none" marker-end="url(#arrow-gray)"/>')
text(756, 738, "旁注 · 全量首锚", 10, "#334155", "start")

A("</svg>")

out = Path(__file__).parent / "02-anchor-and-query-flow.svg"
out.write_text("\n".join(L), encoding="utf-8")
ET.parse(out)  # XML 校验,失败会抛异常
print(f"OK {out} ({out.stat().st_size} bytes)")
