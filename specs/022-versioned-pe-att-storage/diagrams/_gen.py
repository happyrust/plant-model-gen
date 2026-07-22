# -*- coding: ascii -*-
# Generator for 01-storage-architecture.svg (spec 022). ASCII-only source;
# all CJK text is \u-escaped so Windows codepage issues cannot corrupt it.
import os

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "01-storage-architecture.svg")

# ---- text constants (Chinese via \u escapes) ----
T_TITLE = "Spec 022 \u00b7 PE/ATT \u7248\u672c\u5316\u5b58\u50a8\u603b\u4f53\u67b6\u6784"
T_SUB = "SurrealDB fork dev-3.1 \u00b7 RocksDB MVCC\uff08user-defined timestamps\uff09 \u00b7 specs/022-versioned-pe-att-storage"
T_LEGEND = "\u56fe\u4f8b"
L1 = "\u6570\u636e\u843d\u5e93\u5199\u5165\uff08\u5168\u91cf / \u589e\u91cf\uff09"
L2 = "\u951a\u70b9\u56fa\u5316\uff08\u6210\u529f\u624d\u5199\uff09"
L3 = "\u5386\u53f2\u67e5\u8be2\uff08VERSION \u8bfb\uff09"
L4 = "\u7a97\u53e3\u5916\u5164\u5e95\uff1a\u6e90 db \u91cd\u626b"
L5 = "GC \u6c34\u4f4d\u7ebf\uff08retention \u56de\u6536\uff09"
LAYER1 = "\u6570\u636e\u6e90\u5c42"
SRC_TITLE = "PDMS \u6e90 .db \u6587\u4ef6"
SRC_SUB = "\u957f\u671f\u4fdd\u7559 \u00b7 retention \u7a97\u53e3\u5916\u5386\u53f2\u7684\u6700\u7ec8\u5164\u5e95"
ARROW_FULL = "sync_pdms \u5168\u91cf\u91cd\u704c\uff08\u91cd\u5efa/\u5207\u6362\uff09"
ARROW_INCR = "incremental-sesno \u589e\u91cf\u89e3\u6790"
ANCHOR_NOTE = "\u843d\u5e93\u6210\u529f\u6536\u5c3e\u56fa\u5316\u951a\u70b9\uff08\u5931\u8d25\u4e0d\u5199\uff09"
LAYER2 = "\u4e3b\u5b58\u50a8\u5c42"
SUL_TITLE = "SUL_DB \u9879\u76ee\u4e3b\u5e93\uff08SurrealDB fork \u00b7 RocksDB MVCC\uff09"
CONN = "rocksdb://&lt;dir&gt;?versioned=true&amp;retention=90d"
PE_T = "PE \u8868"
NOUN_T = "noun \u8868"
ATT_T = "ATT_UDA \u8868"
MVCC_NOTE = "\u5386\u53f2\u7531 MVCC \u900f\u660e\u4fdd\u7559 \u00b7 \u4e0d\u65b0\u589e\u5b57\u6bb5 \u00b7 \u786c DELETE \u53ef\u56de\u6eaf"
ANCHOR_BRIDGE = "sesno \u2194 \u65f6\u95f4\u6233\u552f\u4e00\u6865\u6881"
GC_LABEL = "retention=90d \u2192 GC \u6c34\u4f4d\u7ebf full_history_ts_low"
GC_NOTE = "GC \u4ee5 60s \u7c92\u5ea6\u63a8\u8fdb \u00b7 \u56de\u6536\u7a97\u53e3\u5916\u5386\u53f2\u517c\u987e\u78c1\u76d8\uff08\u9ed8\u8ba4 retention=0 \u65e0\u9650\u4fdd\u7559\uff09"
COEX = "\u5171\u5b58\u4e0e\u5206\u5de5"
MK_TITLE = "MODEL_KV \u72ec\u7acb\u5b9e\u4f8b\uff08\u539f\u8bbe\u8ba1\uff09"
MK_SUB = "\u627f\u63a5\u6a21\u578b\u9ad8\u9891\u5199 inst_relate / mesh \u7b49"
MK_WARN1 = "\u203b 2026-07-16 \u66f4\u65b0\uff1a\u5206\u79bb\u673a\u5236\u5df2\u6574\u4f53\u79fb\u9664"
MK_WARN2 = "\u6a21\u578b\u8868\u4e0e PE/ATT \u540c\u5e93\u4e00\u5e76\u7248\u672c\u5316\uff08\u51b3\u7b56 1\uff09"
DL_TITLE = "DuckLake release \u4ea4\u4ed8\u5b58\u6863"
DL_SUB1 = "\u4ea4\u4ed8\u7269\u4e0d\u53ef\u53d8\u5b58\u6863\uff08\u7c97\u7c92\u5ea6 \u00b7 \u957f\u671f\u4fdd\u7559\uff09"
DL_SUB2 = "\u4e0e versioned \u5386\u53f2\uff08\u7ec6\u7c92\u5ea6 \u00b7 retention \u7a97\u53e3\uff09"
DL_SUB3 = "\u6b63\u4ea4\u5206\u5de5 \u00b7 \u5171\u5b58\u4e92\u8865"
LAYER3 = "\u6d88\u8d39\u5c42"
CLI_TITLE = "CLI\uff1amodel-version history"
CLI_SUB = "snapshot / timeline / diff\uff08\u5747\u652f\u6301 --json\uff09"
VQ_TITLE = "rs-core version_query \u5c01\u88c5"
VQ_SUB1 = "sesno\u2192\u65f6\u95f4\u6233\u6362\u7b97\uff08resolve_anchor \u6700\u8fd1\u4e0d\u5927\u4e8e\u56de\u9000\uff09"
VQ_SUB2 = "VERSION \u67e5\u8be2\u62fc\u63a5 \u00b7 GC \u8d8a\u754c InvalidArgument\u2192HistoryExpired"
Q1 = "\u2460 \u951a\u70b9\u6362\u7b97 sesno\u2192$t"
Q2 = "\u2461 SELECT \u2026 VERSION $t"
EXPIRED_NOTE = "sesno \u8d85\u51fa retention \u7a97\u53e3 \u2192 HistoryExpired \u2192 \u8d70\u5de6\u4fa7\u7070\u865a\u7ebf\u56de PDMS \u6e90\u6587\u4ef6\u91cd\u626b"
FOOTER = "\u4f9d\u636e spec.md / plan.md / tasks.md / ops-notes.md \u7ed8\u5236 \u00b7 2026-07-18"

L = []
A = L.append
A('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 640">')
A('  <style>')
A("    text { font-family: 'Microsoft YaHei', 'PingFang SC', 'Noto Sans CJK SC', sans-serif; }")
A("    .mono { font-family: Consolas, 'Courier New', monospace; }")
A('  </style>')
A('  <defs>')
A('    <marker id="aBlue" viewBox="0 0 10 7" refX="9" refY="3.5" markerWidth="10" markerHeight="7" orient="auto">')
A('      <path d="M0,0 L10,3.5 L0,7 Z" fill="#2563eb"/>')
A('    </marker>')
A('    <marker id="aGreen" viewBox="0 0 10 7" refX="9" refY="3.5" markerWidth="10" markerHeight="7" orient="auto">')
A('      <path d="M0,0 L10,3.5 L0,7 Z" fill="#059669"/>')
A('    </marker>')
A('    <marker id="aGray" viewBox="0 0 10 7" refX="9" refY="3.5" markerWidth="10" markerHeight="7" orient="auto">')
A('      <path d="M0,0 L10,3.5 L0,7 Z" fill="#6b7280"/>')
A('    </marker>')
A('  </defs>')
A('  <rect x="0" y="0" width="960" height="640" fill="#ffffff"/>')
A('  <text x="480" y="28" text-anchor="middle" font-size="17" font-weight="bold" fill="#1e293b">' + T_TITLE + '</text>')
A('  <text x="480" y="45" text-anchor="middle" font-size="10" fill="#64748b">' + T_SUB + '</text>')
A('  <!-- legend -->')
A('  <rect x="664" y="52" width="256" height="118" rx="8" fill="#ffffff" stroke="#cbd5e1" stroke-width="1"/>')
A('  <text x="676" y="70" font-size="12" font-weight="bold" fill="#475569">' + T_LEGEND + '</text>')
A('  <line x1="676" y1="84" x2="712" y2="84" stroke="#2563eb" stroke-width="2" marker-end="url(#aBlue)"/>')
A('  <text x="720" y="88" font-size="10" fill="#334155">' + L1 + '</text>')
A('  <line x1="676" y1="101" x2="712" y2="101" stroke="#059669" stroke-width="1.6" stroke-dasharray="5,3" marker-end="url(#aGreen)"/>')
A('  <text x="720" y="105" font-size="10" fill="#334155">' + L2 + '</text>')
A('  <line x1="676" y1="118" x2="712" y2="118" stroke="#059669" stroke-width="1.8" marker-end="url(#aGreen)"/>')
A('  <text x="720" y="122" font-size="10" fill="#334155">' + L3 + '</text>')
A('  <line x1="676" y1="135" x2="712" y2="135" stroke="#6b7280" stroke-width="1.5" stroke-dasharray="5,3" marker-end="url(#aGray)"/>')
A('  <text x="720" y="139" font-size="10" fill="#334155">' + L4 + '</text>')
A('  <line x1="676" y1="152" x2="712" y2="152" stroke="#ea580c" stroke-width="1.6" stroke-dasharray="7,4"/>')
A('  <text x="720" y="156" font-size="10" fill="#334155">' + L5 + '</text>')
A('  <!-- layer containers (drawn first so labels can mask their borders) -->')
A('  <rect x="40" y="52" width="600" height="90" rx="10" fill="none" stroke="#94a3b8" stroke-width="1.2" stroke-dasharray="6,4"/>')
A('  <text x="52" y="70" font-size="12" font-weight="bold" fill="#64748b">' + LAYER1 + '</text>')
A('  <rect x="40" y="190" width="880" height="256" rx="10" fill="none" stroke="#94a3b8" stroke-width="1.2" stroke-dasharray="6,4"/>')
A('  <text x="52" y="208" font-size="12" font-weight="bold" fill="#64748b">' + LAYER2 + '</text>')
A('  <rect x="600" y="214" width="304" height="216" rx="10" fill="none" stroke="#94a3b8" stroke-width="1.2" stroke-dasharray="6,4"/>')
A('  <text x="612" y="232" font-size="11" font-weight="bold" fill="#64748b">' + COEX + '</text>')
A('  <rect x="40" y="470" width="880" height="126" rx="10" fill="none" stroke="#94a3b8" stroke-width="1.2" stroke-dasharray="6,4"/>')
A('  <text x="52" y="488" font-size="12" font-weight="bold" fill="#64748b">' + LAYER3 + '</text>')
A('  <!-- data source layer -->')
A('  <rect x="210" y="72" width="270" height="54" rx="8" fill="#fff7ed" stroke="#f59e0b" stroke-width="1.5"/>')
A('  <text x="345" y="93" text-anchor="middle" font-size="13" font-weight="bold" fill="#9a3412">' + SRC_TITLE + '</text>')
A('  <text x="345" y="112" text-anchor="middle" font-size="10" fill="#b45309">' + SRC_SUB + '</text>')
A('  <!-- ingest arrows -->')
A('  <line x1="280" y1="126" x2="280" y2="212" stroke="#2563eb" stroke-width="2" marker-end="url(#aBlue)"/>')
A('  <rect x="180" y="148" width="200" height="16" fill="#ffffff" opacity="0.95"/>')
A('  <text x="280" y="160" text-anchor="middle" font-size="10.5" fill="#1d4ed8">' + ARROW_FULL + '</text>')
A('  <line x1="410" y1="126" x2="410" y2="212" stroke="#2563eb" stroke-width="2" marker-end="url(#aBlue)"/>')
A('  <rect x="332" y="184" width="156" height="16" fill="#ffffff" opacity="0.95"/>')
A('  <text x="410" y="196" text-anchor="middle" font-size="10.5" fill="#1d4ed8">' + ARROW_INCR + '</text>')
A('  <!-- anchor write (success only) -->')
A('  <circle cx="410" cy="172" r="3" fill="#059669"/>')
A('  <path d="M410,172 H500 V264" fill="none" stroke="#059669" stroke-width="1.6" stroke-dasharray="5,3" marker-end="url(#aGreen)"/>')
A('  <rect x="500" y="174" width="192" height="16" fill="#ffffff" opacity="0.95"/>')
A('  <text x="504" y="186" font-size="10.5" fill="#047857">' + ANCHOR_NOTE + '</text>')
A('  <!-- main storage layer -->')
A('  <rect x="60" y="214" width="520" height="216" rx="10" fill="#eff6ff" stroke="#2563eb" stroke-width="1.6"/>')
A('  <text x="320" y="236" text-anchor="middle" font-size="13" font-weight="bold" fill="#1e3a8a">' + SUL_TITLE + '</text>')
A('  <text x="320" y="254" text-anchor="middle" font-size="10.5" fill="#1d4ed8" class="mono">' + CONN + '</text>')
A('  <!-- table cylinders -->')
cyl = [(123, PE_T), (242, NOUN_T), (352, ATT_T)]
for cx, name in cyl:
    x1 = cx - 47.5
    x2 = cx + 47.5
    A('  <path d="M%s,272 V326 A47.5,8 0 0 0 %s,326 V272" fill="#dbeafe" stroke="#3b82f6" stroke-width="1.4"/>' % (x1, x2))
    A('  <ellipse cx="%s" cy="272" rx="47.5" ry="8" fill="#bfdbfe" stroke="#3b82f6" stroke-width="1.4"/>' % cx)
    A('  <text x="%s" y="305" text-anchor="middle" font-size="12" font-weight="bold" fill="#1e40af">%s</text>' % (cx, name))
A('  <text x="140" y="358" font-size="10" fill="#475569">' + MVCC_NOTE + '</text>')
A('  <!-- anchor table -->')
A('  <rect x="430" y="266" width="140" height="80" rx="6" fill="#fef9c3" stroke="#ca8a04" stroke-width="1.5"/>')
A('  <text x="500" y="281" text-anchor="middle" font-size="10.5" font-weight="bold" fill="#854d0e" class="mono">sesno_version_anchor</text>')
A('  <text x="500" y="295" text-anchor="middle" font-size="9.5" fill="#a16207" class="mono">{ dbnum, sesno,</text>')
A('  <text x="500" y="307" text-anchor="middle" font-size="9.5" fill="#a16207" class="mono">anchored_at, source }</text>')
A('  <text x="500" y="320" text-anchor="middle" font-size="9.5" fill="#a16207" class="mono">source: full | incr</text>')
A('  <text x="500" y="337" text-anchor="middle" font-size="10" font-weight="bold" fill="#b45309">' + ANCHOR_BRIDGE + '</text>')
A('  <!-- GC watermark -->')
A('  <text x="488" y="384" text-anchor="end" font-size="9.5" font-weight="bold" fill="#c2410c">' + GC_LABEL + '</text>')
A('  <line x1="76" y1="394" x2="564" y2="394" stroke="#ea580c" stroke-width="1.6" stroke-dasharray="7,4"/>')
A('  <text x="140" y="412" font-size="10" fill="#9a3412">' + GC_NOTE + '</text>')
A('  <!-- coexistence -->')
A('  <rect x="614" y="244" width="276" height="84" rx="8" fill="#f9fafb" stroke="#9ca3af" stroke-width="1.3" stroke-dasharray="5,3"/>')
A('  <text x="752" y="262" text-anchor="middle" font-size="11.5" font-weight="bold" fill="#6b7280">' + MK_TITLE + '</text>')
A('  <text x="752" y="278" text-anchor="middle" font-size="9.5" fill="#6b7280">' + MK_SUB + '</text>')
A('  <text x="752" y="296" text-anchor="middle" font-size="9.5" font-weight="bold" fill="#dc2626">' + MK_WARN1 + '</text>')
A('  <text x="752" y="312" text-anchor="middle" font-size="9.5" fill="#dc2626">' + MK_WARN2 + '</text>')
A('  <rect x="614" y="340" width="276" height="82" rx="8" fill="#f0fdf4" stroke="#16a34a" stroke-width="1.4"/>')
A('  <text x="752" y="358" text-anchor="middle" font-size="11.5" font-weight="bold" fill="#166534">' + DL_TITLE + '</text>')
A('  <text x="752" y="374" text-anchor="middle" font-size="9.5" fill="#15803d">' + DL_SUB1 + '</text>')
A('  <text x="752" y="390" text-anchor="middle" font-size="9.5" fill="#15803d">' + DL_SUB2 + '</text>')
A('  <text x="752" y="406" text-anchor="middle" font-size="9.5" font-weight="bold" fill="#15803d">' + DL_SUB3 + '</text>')
A('  <!-- consumer layer -->')
A('  <rect x="70" y="500" width="340" height="62" rx="8" fill="#faf5ff" stroke="#7c3aed" stroke-width="1.5"/>')
A('  <text x="240" y="523" text-anchor="middle" font-size="12.5" font-weight="bold" fill="#5b21b6">' + CLI_TITLE + '</text>')
A('  <text x="240" y="543" text-anchor="middle" font-size="10" fill="#6d28d9">' + CLI_SUB + '</text>')
A('  <rect x="480" y="500" width="380" height="62" rx="8" fill="#ede9fe" stroke="#7c3aed" stroke-width="1.5"/>')
A('  <text x="670" y="518" text-anchor="middle" font-size="12" font-weight="bold" fill="#5b21b6">' + VQ_TITLE + '</text>')
A('  <text x="670" y="535" text-anchor="middle" font-size="9.5" fill="#6d28d9">' + VQ_SUB1 + '</text>')
A('  <text x="670" y="552" text-anchor="middle" font-size="9.5" fill="#6d28d9">' + VQ_SUB2 + '</text>')
A('  <line x1="410" y1="531" x2="478" y2="531" stroke="#2563eb" stroke-width="2" marker-end="url(#aBlue)"/>')
A('  <text x="70" y="586" font-size="9.5" fill="#6b7280">' + EXPIRED_NOTE + '</text>')
A('  <!-- history query arrows -->')
A('  <path d="M520,500 V484 H123 V336" fill="none" stroke="#059669" stroke-width="1.8" marker-end="url(#aGreen)"/>')
A('  <rect x="127" y="450" width="164" height="16" fill="#ffffff" opacity="0.95"/>')
A('  <text x="131" y="462" font-size="10" fill="#047857">' + Q2 + '</text>')
A('  <path d="M540,500 V348" fill="none" stroke="#059669" stroke-width="1.8" marker-end="url(#aGreen)"/>')
A('  <rect x="546" y="450" width="150" height="16" fill="#ffffff" opacity="0.95"/>')
A('  <text x="550" y="462" font-size="10" fill="#047857">' + Q1 + '</text>')
A('  <!-- expired fallback to source files -->')
A('  <path d="M70,548 H30 V98 H208" fill="none" stroke="#6b7280" stroke-width="1.4" stroke-dasharray="5,3" marker-end="url(#aGray)"/>')
A('  <text x="920" y="630" text-anchor="end" font-size="9" fill="#94a3b8">' + FOOTER + '</text>')
A('</svg>')

with open(OUT, "w", encoding="utf-8", newline="\n") as f:
    f.write("\n".join(L) + "\n")
print("wrote", OUT, len(L), "lines")
