# SigMap Query Context
Generated: 2026-06-08T18:27:52.174Z

## .trellis\scripts\add_session.py
```
def get_latest_journal_info(dev_dir: Path) → tuple[Path | None, int, int]  # Get latest journal file info
def get_current_session(index_file: Path) → int  # Get current session number from index
def count_journal_files(dev_dir: Path, active_num: int) → str  # Count journal files and return table rows
def create_new_journal_file(dev_dir: Path, num: int, developer: str, today: str, max_lines: int) → Path  # Create a new journal file
def generate_session_content(session_num: int, title: str, commit: str, summary: str, extra_content: str, today: str, package: str | None, branch: str | None) → str  # Generate session content
def update_index(index_file: Path, dev_dir: Path, title: str, commit: str, new_session: int, active_file: str, today: str, branch: str | None) → bool  # Update index
def main() → int  # CLI entry point
```

## .cursor\skills\trellis-meta\references\customize-local\add-project-local-conventions.md
```
h1 Add Project-Local Conventions
h2 Where To Put Things
h2 Create A Project-Local Skill
h1 Trellis Local
h2 Local Scope
h2 Custom Workflow Rules
h2 Local Hook Changes
h2 Local Agent Changes
h2 Write To `.trellis/spec/`
h2 Make The Current Task Use New Conventions
h2 Do Not Store Project-Private Rules In `trellis-meta`
code-fence text
code-fence plain
code-fence md
code-fence bash
```

## .cursor\skills\trellis-update-spec\SKILL.md
```
h1 Update Code-Spec - Capture Executable Contracts
h2 Code-Spec First Rule (CRITICAL)
h3 Mandatory Triggers
h3 Mandatory Output (7 Sections)
h2 When to Update Code-Specs
h2 Spec Structure Overview
h3 CRITICAL: Code-Spec vs Guide - Know the Difference
h2 Update Process
h3 Step 1: Identify What You Learned
h3 Step 2: Classify the Update Type
h3 Step 3: Read the Target Code-Spec
h3 Step 4: Make the Update
h3 Step 5: Update the Index (if needed)
h2 Update Templates
h3 Mandatory Template for Infra/Cross-Layer Work
h2 Scenario: <name>
h3 1. Scope / Trigger
h3 2. Signatures
h3 3. Contracts
h3 4. Validation & Error Matrix
```

## .worktrees\model-persistence-trait\docs\development\model-writer-storage\01-current-surreal-write-contract.md
```
h1 Current SurrealDB Write Contract
h2 Current behavior
h2 Contract principles
h2 Parity matrix
h2 Backend parity requirements
```

## .trellis\scripts\common\config.py
```
def parse_simple_yaml(content: str) → dict  # Parse simple YAML with nested dict support (no dependencies)
def get_session_commit_message(repo_root: Path | None) → str  # Get the commit message for auto-committing session records
def get_max_journal_lines(repo_root: Path | None) → int  # Get the maximum lines per journal file
def get_session_auto_commit(repo_root: Path | None) → bool
def get_hooks(event: str, repo_root: Path | None) → list[str]  # Get hook commands for a lifecycle event
def get_packages(repo_root: Path | None) → dict[str, dict] | None  # Get monorepo package declarations
def get_default_package(repo_root: Path | None) → str | None  # Get the default package name from config
def get_submodule_packages(repo_root: Path | None) → dict[str, str]  # Get packages that are git submodules
def get_git_packages(repo_root: Path | None) → dict[str, str]  # Get packages that have their own independent git repository
def is_monorepo(repo_root: Path | None) → bool  # Check if the project is configured as a monorepo (has packag
def get_spec_base(package: str | None, repo_root: Path | None) → str  # Get the spec directory base path relative to
def validate_package(package: str, repo_root: Path | None) → bool  # Check if a package name is valid in this project
def resolve_package(task_package: str | None, repo_root: Path | None) → str | None  # Resolve package from inferred sources with validation
def get_spec_scope(repo_root: Path | None) → list[str] | str | None  # Get session
```
