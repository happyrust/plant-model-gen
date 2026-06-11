# BRAN 24381_145018 依赖数据 — 新旧解析差异 refno 清单

> 对比基准：8020 老库（旧解析器）vs 按需站点（新解析器，与 e2e 全量一致）。
> 待用户从 E3D 原生（`Q ATT` / `Q <字段>`）提供最正宗参考值。
> E3D refno 格式：`13246_243891` → `=13246/243891`

## A. LEVE 数组截断（8020=[lo,hi]，新=[lo]）— 63 处，全部几何基元

| refno | noun | 8020 | 新解析 | 待确认 E3D 真值 |
|---|---|---|---|---|
| =13246/243926 | SCYL | [8, 10] | [8] | Q LEVE |
| =13246/243923 | LSNO | [0, 10] | [0] | Q LEVE |
| =13246/243933 | LPYR | [0, 10] | [0] | Q LEVE |
| =13246/243938 | SCTO | [0, 10] | [0] | Q LEVE |
| =13246/243929 | SSPH | [0, 10] | [0] | Q LEVE |
| =13246/243974 | SBOX | [0, 10] | [0] | Q LEVE |
| =13246/243922 | LINE | [0, 10] | [0] | Q LEVE |

（其余 56 处同模式：GMSE 13246_243921 全部子基元 + ELBO/OLET/ATTA 元件库基元）

## B. PTCA.PTCD 丢失（8020 有方向编码字符串，新=空）— 26 处

| refno | noun | 8020 PTCD | 新解析 |
|---|---|---|---|
| =13246/243891 | PTCA | '-Z' | （丢失） |
| =13246/243892 | PTCA | （有值） | （丢失） |
| =13246/243893 | PTCA | （有值） | （丢失） |

（GMSE /AA/NI/VALVE/YJUSSARFG01-G 点集与 ELBO/OLET 点集的全部 PTCA，共 26 个：
13246_243891 ~ 243919 区段 + 390328/390329 等）

## C. PTCA.PHEI 丢失 — 1 处

| refno | noun | 8020 | 新解析 |
|---|---|---|---|
| =13246/243891 | PTCA | '223' | （丢失） |

## D. SDTE.SKEY / RTEX 丢失 — 7 处

| refno | noun | 字段 | 8020 | 新解析 |
|---|---|---|---|---|
| =13246/243869 | SDTE | SKEY | 'VGBW' | （丢失） |
| =13246/243869 | SDTE | RTEX | 'ATTRIB NAMN[500 ]' | （丢失） |
| =13246/246609 | SDTE | SKEY | （有值） | （丢失） |
| =13246/246627 | SDTE | SKEY | （有值） | （丢失） |
| =13246/246758 | SDTE | SKEY | （有值） | （丢失） |
| =13246/247163 | SDTE | SKEY | （有值） | （丢失） |
| =13246/247262 | SDTE | SKEY | （有值） | （丢失） |

## E. 数字 0 → 空串（疑似"默认值不落库"，请确认 E3D 真值类型）— 约 110 处

| refno | noun | 字段 | 8020 | 新解析 |
|---|---|---|---|---|
| =13246/243891 | PTCA | PSKE / PURP | 0 / 0 | '' / '' |
| =13246/243890 | PTAX | PSKE / PURP | 0 / 0 | '' / '' |
| =13246/243920 | PTMI | PSKE / PURP | 0 / 0 | '' / '' |
| =13246/243889 | PTSE | GTYP / PURP | 0 / 0 | '' / '' |
| =13246/243921 | GMSE | GTYP / PURP | 0 / 0 | '' / '' |
| =24381/145020 | ATTA | ATTY | 0 | '' |
| =24381/145018 | BRAN | MTOH / FLOW / LNTP / PURP | 0 | ''（部分字段) |
| =13246/465579 | SPEC | LNTP / PURP | 0 | '' |

## F. 表达式文本格式差异（去括号规范化，数值疑似等价但请抽验）— 16 类

| refno | noun | 字段 | 8020 | 新解析 |
|---|---|---|---|---|
| =13246/243893 | PTCA | PZ | ( 0.7 * ATTRIB PARA[7 ] ) | 0.7 * ATTRIB PARA[7 ] |
| =13246/243899 | PTCA | PY | ( ( -( 0.5 * ATTRIB PARA[8 ] ) ) - 53 ) | - (0.5 * ATTRIB PARA[8 ]) - 53 |
| =13246/243943 | SCYL | PHEI | ( 0.5 * ATTRIB PARA[11 ] ) | 0.5 * ATTRIB PARA[11 ] |
| =13246/243967 | LSNO | PBDM | ( ( 0.9 * ATTRIB PARA[4 ] ) + ATTRIB IPAR[1 ] ) | 0.9 * ATTRIB PARA[4 ] + ATTRIB IPAR[1 ] |
| =13246/243967 | LSNO | PTDM | ( ( 1 * ATTRIB PARA[4 ] ) + ATTRIB IPAR[1 ] ) | 1 * ATTRIB PARA[4 ] + ATTRIB IPAR[1 ] |
| =13246/243926 | SCYL | PDIA | ( ATTRIB PARA[8 ] * 1.2 ) | ATTRIB PARA[8 ] * 1.2 |
| =13246/243953 | SCYL | PDIS | ( ATTRIB PARA[7 ] + ( 0.5 * ATTRIB PARA[9 ] ) ) | ATTRIB PARA[7 ] + 0.5 * ATTRIB PARA[9 ] |
| =13246/243937 | LPYR | PBDI / PTDI / PCBT / PCTP | ( 30 - ( 0.5 * ATTRIB IPAR[1 ] ) ) 等 | 30 - 0.5 * ATTRIB IPAR[1 ] 等 |
| =13246/243975 | SBOX | PZLE | ( 305 + ATTRIB IPAR[1 ] ) | 305 + ATTRIB IPAR[1 ] |
| =13246/243954 | LSNO | PBDI / PTDI | ( ATTRIB PARA[7 ] + ( 0.6 * ATTRIB PARA[9 ] ) ) 等 | ATTRIB PARA[7 ] + 0.6 * ATTRIB PARA[9 ] 等 |

---
重点核对优先级建议：
1. **=13246/243891（PTCA）**：一个元素同时覆盖 B(PTCD='-Z') / C(PHEI='223') / E(PSKE/PURP) 三类
2. **=13246/243926（SCYL）**：覆盖 A(LEVE=[8,10]) / F(PDIA 表达式)
3. **=13246/243869（SDTE）**：覆盖 D(SKEY='VGBW', RTEX)
4. **=13246/243899（PTCA）**：F 类中唯一含负号嵌套的表达式（优先验证运算等价性）
