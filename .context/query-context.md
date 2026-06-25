# SigMap Query Context
Generated: 2026-06-25T04:47:54.610Z

## .cursor\agents\trellis-research.md
```
h1 Research Agent
h2 Core Principle
h2 Core Responsibilities
h2 Workflow
h3 Step 1: Resolve Current Task
h3 Step 2: Understand Search Request
h3 Step 3: Execute Search
h3 Step 4: Persist Each Topic
h3 Step 5: Report to Main Agent
h2 Scope Limits (Strict)
h3 Write ALLOWED
h3 Write FORBIDDEN
h2 File Format
h1 Research: <topic>
h2 Findings
h3 Files Found
h3 Code Patterns
h3 External References
h3 Related Specs
h2 Caveats / Not Found
```

## .cursor\hooks\inject-subagent-context.py
```
def find_repo_root(start_path: str) → str | None  # Find git repo root from start_path upwards
def get_current_task(repo_root: str, input_data: dict) → str | None  # Resolve current task directory through the unified active ta
def read_file_content(base_path: str, file_path: str) → str | None  # Read file content, return None if file doesn't exist
def read_directory_contents(base_path: str, dir_path: str, max_files: int) → list[tuple[str, str]]  # Read all
def read_jsonl_entries(base_path: str, jsonl_path: str) → list[tuple[str, str]]
def get_agent_context(repo_root: str, task_dir: str, agent_type: str) → str  # Get context from {agent_type}
def get_implement_context(repo_root: str, task_dir: str) → str  # Complete context for Implement Agent
def get_check_context(repo_root: str, task_dir: str) → str  # Context for Check Agent: check
def get_finish_context(repo_root: str, task_dir: str) → str  # Context for Finish phase: reuses check
def build_implement_prompt(original_prompt: str, context: str) → str  # Build complete prompt for Implement
def build_check_prompt(original_prompt: str, context: str) → str  # Build complete prompt for Check
def build_finish_prompt(original_prompt: str, context: str) → str  # Build complete prompt for Finish (final check before PR)
def get_research_context(repo_root: str, task_dir: str | None) → str  # Context for Research Agent — project structure overview for
def build_research_prompt(original_prompt: str, context: str) → str  # Build complete prompt for Research
def main()
```

## .cursor\skills\planning-with-files\examples.md
```
h1 Examples: Planning with Files in Action
h2 Example 1: Research Task
h3 Loop 1: Create Plan
h1 Task Plan: Morning Exercise Benefits Research
h2 Goal
h2 Phases
h2 Key Questions
h2 Status
h3 Loop 2: Research
h3 Loop 3: Synthesize
h3 Loop 4: Deliver
h2 Example 2: Bug Fix Task
h3 task_plan.md
h1 Task Plan: Fix Login Bug
h2 Decisions Made
h2 Errors Encountered
h2 Example 3: Feature Development
h3 The 3-File Pattern in Action
h1 Task Plan: Dark Mode Toggle
h1 Findings: Dark Mode Implementation
```

## .cursor\skills\trellis-meta\references\local-architecture\workflow.md
```
h1 Local Workflow System
h2 File Responsibilities
h2 Current Phase Model
h2 Skill Routing
h2 Workflow-State Prompt Blocks
h2 Local Modification Patterns
h2 Relationship To Platform Files
code-fence text
code-fence plain
```

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
