# audit_anchor_continuity — sesno_version_anchor 锚点链连续性审计结果

- 生成时间：2026-07-20 11:30:15 +08:00（由 scripts/smoke/anchor_continuity_audit.ps1 生成，重跑会覆盖本文件）
- 实例：`http://127.0.0.1:8030`（HTTP POST /sql，Basic root/***）
- 环境：`ns=vc_verify` `db=continuity_gate`
- 审计 SQL：`db-data/audit_anchor_continuity.surql`（判定规则与用法见该文件头注释）

## 判定

**FAIL：发现 1 条断链可疑项——该 dbnum 历史存在未采集的 sesno 区间；修复口径=对该 dbnum 全量重灌（锚点不可变，不做补洞回填）**

## [1] 锚点链全量（dbnum, sesno 升序，共 10 条）

```json
[
    {
        "anchored_at":  "2026-07-20T03:27:04.660331600Z",
        "dbnum":  9301,
        "from_sesno":  10,
        "has_fingerprint":  true,
        "sesno":  10,
        "source":  "full"
    },
    {
        "anchored_at":  "2026-07-20T03:27:04.660929600Z",
        "dbnum":  9301,
        "from_sesno":  11,
        "has_fingerprint":  true,
        "sesno":  12,
        "source":  "incremental"
    },
    {
        "anchored_at":  "2026-07-20T03:27:04.661415100Z",
        "dbnum":  9301,
        "from_sesno":  13,
        "has_fingerprint":  true,
        "sesno":  15,
        "source":  "incremental"
    },
    {
        "anchored_at":  "2026-07-20T03:27:04.661891800Z",
        "dbnum":  9301,
        "from_sesno":  17,
        "has_fingerprint":  true,
        "sesno":  20,
        "source":  "incremental"
    },
    {
        "anchored_at":  "2026-07-20T03:27:04.662346100Z",
        "dbnum":  9301,
        "from_sesno":  null,
        "has_fingerprint":  false,
        "sesno":  25,
        "source":  "incremental"
    },
    {
        "anchored_at":  "2026-07-20T03:27:04.662816400Z",
        "dbnum":  9301,
        "from_sesno":  26,
        "has_fingerprint":  true,
        "sesno":  30,
        "source":  "incremental"
    },
    {
        "anchored_at":  "2026-07-20T03:27:04.663284300Z",
        "dbnum":  9302,
        "from_sesno":  3,
        "has_fingerprint":  true,
        "sesno":  5,
        "source":  "incremental"
    },
    {
        "anchored_at":  "2026-07-20T03:27:04.663747500Z",
        "dbnum":  9303,
        "from_sesno":  3,
        "has_fingerprint":  true,
        "sesno":  4,
        "source":  "incremental"
    },
    {
        "anchored_at":  "2026-07-20T03:27:04.664205200Z",
        "dbnum":  9303,
        "from_sesno":  8,
        "has_fingerprint":  true,
        "sesno":  8,
        "source":  "full"
    },
    {
        "anchored_at":  "2026-07-20T03:27:04.664666700Z",
        "dbnum":  9303,
        "from_sesno":  9,
        "has_fingerprint":  true,
        "sesno":  9,
        "source":  "incremental"
    }
]
```

## [2] 断链可疑项（from_sesno 存在、source != 'full'、from_sesno != 前一锚点 sesno + 1，共 1 条）

```json
{
    "dbnum":  9301,
    "expected_from":  16,
    "from_sesno":  17,
    "prev_sesno":  15,
    "sesno":  20,
    "source":  "incremental"
}
```

## 修复口径

历史断链只能对该 dbnum **全量重灌**（重建锚点链基线）；锚点是 create-once 不可变发布记录，
不做"补洞"式回填——详见 `specs/022-versioned-pe-att-storage/ops-notes.md` 第四节。
