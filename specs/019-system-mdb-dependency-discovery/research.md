# Research: System-Library MDB Dependency Discovery

## Decision: Treat Directory Scan As Candidate Discovery Only

**Rationale**: Directory names and folder layout are not authoritative for MDB membership. Operators may have renamed folders, copied subsets of projects, or placed several E3D projects under one parent. A scan can find candidate DB files, but it cannot prove which project paths are dependencies until system-library content is parsed.

**Alternatives considered**:

- Infer dependency project paths from folder names matching MDB/project names. Rejected because it is brittle and fails on renamed or duplicated folders.
- Require users to manually enter all projects. Rejected because it preserves the current friction and does not satisfy quick deploy.
- Build a full parse/index first. Rejected for quick-deploy discovery because it is too slow and broad; only system-library facts are needed at this stage.

## Decision: Parse SYST/GLOB/GLB As MDB Source Libraries

**Rationale**: The existing implementation parsed SYST only. The user clarified that SYST and GLOB-like system files must be parsed before dependency paths are considered known. SYST remains the highest-priority source because it is the current MDB authority path, while GLOB/GLB are treated as supported system-library sources that can also provide evidence or future compatibility.

**Alternatives considered**:

- Keep SYST-only behavior. Rejected because it contradicts the clarified requirement and leaves GLOB/GLB invisible to MDB discovery.
- Parse every DB file fully. Rejected because it is unnecessary and would make interactive discovery slower.
- Parse only GLOB/GLB. Rejected because current known MDB enumeration comes from SYST and must remain compatible.

## Decision: Preserve Source Evidence In Candidate Responses

**Rationale**: Operators and maintainers need to know which system-library file proved the MDB and where each member DB was located. Without source evidence, missing or ambiguous dependencies are hard to diagnose.

**Alternatives considered**:

- Return only the resolved target DB. Rejected because it hides dependency completeness and prevents useful error reporting.
- Return raw parse structures. Rejected because it leaks internal parser detail and is harder for the admin UI to consume.
- Keep only `syst_file`. Rejected as misleading once GLOB/GLB can participate; retained only as a compatibility alias.

## Decision: Fail Before Site Creation On Missing Or Ambiguous Dependencies

**Rationale**: A quick-deploy configuration created from incomplete dependencies leads to downstream parse/generate failures such as missing CATA or BRAN output. Failing early gives the operator a direct action: adjust the search root, project collection, dbnum, or db_file.

**Alternatives considered**:

- Create the site and warn. Rejected because it makes the broken configuration look valid.
- Fall back to dbfile-only quick deploy. Rejected because it hides that the MDB dependency closure was not resolved.
- Allow partial deploy for speed. Rejected because the user's problem is dependency correctness, not raw site creation speed.

## Decision: Keep Drawer Path Fill Non-Authoritative

**Rationale**: The drawer helper improves typing speed by composing `root + project/MDB folder name`, but it cannot prove dependency completeness. Keeping it non-authoritative avoids confusing a path convenience with semantic discovery.

**Alternatives considered**:

- Run full MDB discovery directly on every drawer path fill. Rejected because the drawer helper should stay lightweight and predictable.
- Remove drawer helper. Rejected because it still helps users quickly populate project paths and scan roots.

## Decision: Validation Through Running Service And Artifact Inspection

**Rationale**: Repository rules prohibit `cargo test` for `web_server`; verification must use compile checks, HTTP/POST against a running service, CLI/json output, and generated artifacts.

**Alternatives considered**:

- Add Rust unit tests. Rejected by project rule.
- Validate only with `cargo check`. Rejected because it proves type correctness but not end-to-end discovery behavior.
