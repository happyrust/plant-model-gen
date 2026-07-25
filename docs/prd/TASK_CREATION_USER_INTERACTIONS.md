# Task Creation Feature - User Interaction Enumeration

## Overview
This document enumerates ALL possible user interactions with the enhanced task creation wizard for model generation tasks in plant3d-web. The wizard supports dbnum/refno selection, noun filtering, and limit parameters.

---

## 1. BASIC TASK CREATION FLOW

### 1.1 Initial Access
**User Action:** Navigate to task creation wizard
- Click "进入向导" button from main page
- Navigate to `/wizard` route
- Click "Create Task" from task list

**Expected Outcome:** 
- Wizard opens showing Step 1 (Task Type Selection)
- UI displays 3-step progress indicator
- Form fields are empty/default state

**Evidence:**
- Screenshot: Wizard page loads with step indicator
- Console: No JavaScript errors
- Network: GET `/wizard` returns 200

### 1.2 Task Type Selection
**User Action:** Select task type from dropdown
- Options: DataGeneration, ModelGeneration, SpatialTreeGeneration, FullSync, IncrementalSync, ModelExport
- Focus on: **DataGeneration** (the new enhanced type)

**Expected Outcome:**
- Dropdown shows all available task types
- Selection updates form state
- Next step button becomes enabled

**Evidence:**
- Screenshot: Dropdown expanded showing options
- DevTools: Alpine.js state shows `taskRequest.task_type = "DataGeneration"`

### 1.3 Task Name Entry
**User Action:** Enter task name in text field
- Type alphanumeric characters
- Include special characters (spaces, hyphens, underscores)
- Leave empty (validation test)

**Expected Outcome:**
- Text appears in input field
- Character count updates (if implemented)
- Validation triggers on blur/submit

**Evidence:**
- Screenshot: Filled task name field
- Console: No validation errors for valid names
- Network: POST validation request (if real-time validation exists)

---

## 2. TARGET SELECTION (dbnum OR refno)

### 2.1 Choose dbnum
**User Action:** Select "Database Number" radio button and enter dbnum
- Click dbnum radio option
- Enter valid dbnum (e.g., 7997, 1516)
- Enter invalid dbnum (e.g., 0, negative, non-numeric, 99999999)

**Expected Outcome:**
- dbnum input field becomes enabled
- refno input field becomes disabled/hidden
- Validation accepts valid dbnums (positive integers)
- Validation rejects invalid values with error message

**Evidence:**
- Screenshot: dbnum field active, refno field disabled
- Console: Validation error for invalid dbnum
- Network: POST `/api/tasks/create` with `parameters.dbnum = 7997`

### 2.2 Choose refno
**User Action:** Select "Reference Number" radio button and enter refno
- Click refno radio option
- Enter valid refno formats:
  - Simple: "123"
  - Hierarchical: "1/456"
  - Deep hierarchy: "1/2/3/456"
- Enter invalid refno (empty, special chars like "abc", "1//2")

**Expected Outcome:**
- refno input field becomes enabled
- dbnum input field becomes disabled/hidden
- Validation accepts valid refno formats
- Validation rejects invalid formats with error message

**Evidence:**
- Screenshot: refno field active, dbnum field disabled
- Console: Validation error for invalid refno
- Network: POST `/api/tasks/create` with `parameters.refno = "1/456"`

### 2.3 Both dbnum AND refno (Edge Case)
**User Action:** Attempt to provide both dbnum and refno
- Fill dbnum field
- Switch to refno and fill it
- Submit form

**Expected Outcome:**
- UI enforces mutual exclusivity (radio buttons)
- Backend validates only one is provided
- If both sent, backend returns validation error

**Evidence:**
- Console: "Only one of dbnum or refno can be specified"
- Network: 400 Bad Request with error details
- Screenshot: Error message displayed

### 2.4 Neither dbnum NOR refno (Edge Case)
**User Action:** Leave both fields empty and submit
- Skip target selection entirely
- Click Next/Submit

**Expected Outcome:**
- Frontend validation prevents submission
- Error message: "Please specify either dbnum or refno"
- Submit button disabled or validation error shown

**Evidence:**
- Screenshot: Validation error message
- Console: Form validation failure
- Network: No POST request sent (blocked by frontend)

---

## 3. NOUN FILTERING

### 3.1 Add Single Noun
**User Action:** Select one noun type from dropdown/checkbox list
- Options: BRAN, HANG, PANE, TUBI, VALV, ELBO, etc.
- Click/select "BRAN"

**Expected Outcome:**
- Selected noun appears in "Selected Nouns" list
- Noun is added to `parameters.nouns` array
- Visual indicator shows selection (chip/tag)

**Evidence:**
- Screenshot: "BRAN" chip displayed
- DevTools: `parameters.nouns = ["BRAN"]`
- Network: POST payload includes `"nouns": ["BRAN"]`

### 3.2 Add Multiple Nouns
**User Action:** Select multiple noun types
- Select BRAN, HANG, PANE, TUBI
- Use multi-select dropdown or click multiple checkboxes

**Expected Outcome:**
- All selected nouns appear in list
- Array contains all selections: `["BRAN", "HANG", "PANE", "TUBI"]`
- Order preserved or alphabetically sorted

**Evidence:**
- Screenshot: Multiple noun chips displayed
- DevTools: `parameters.nouns = ["BRAN", "HANG", "PANE", "TUBI"]`

### 3.3 Remove Noun
**User Action:** Click remove/delete icon on selected noun chip
- Click "×" button on "BRAN" chip

**Expected Outcome:**
- Noun removed from selected list
- Array updated: `["HANG", "PANE", "TUBI"]`
- Visual chip disappears

**Evidence:**
- Screenshot: "BRAN" chip removed
- DevTools: Updated nouns array without "BRAN"

### 3.4 Empty Nouns (All Types)
**User Action:** Leave nouns selection empty
- Don't select any nouns
- Submit form

**Expected Outcome:**
- Backend interprets empty array as "all noun types"
- Task processes all equipment types
- No validation error (empty is valid = all)

**Evidence:**
- Network: POST with `"nouns": []` or field omitted
- Backend log: "Processing all noun types"
- Task executes successfully for all types

### 3.5 Unknown/Invalid Noun (Edge Case)
**User Action:** Manually inject unknown noun via DevTools
- Set `parameters.nouns = ["INVALID_NOUN", "BRAN"]`
- Submit form

**Expected Outcome:**
- Backend validates noun types against known list
- Returns validation error: "Unknown noun type: INVALID_NOUN"
- Or: Backend ignores unknown nouns and processes valid ones

**Evidence:**
- Console: Validation error message
- Network: 400 Bad Request or warning in response
- Backend log: Validation failure or warning

---

## 4. LIMIT PARAMETER

### 4.1 Enter Valid Limit
**User Action:** Enter positive integer in limit field
- Type "100"
- Type "1"
- Type "10000"

**Expected Outcome:**
- Value accepted
- Task will process up to N items
- Field shows entered value

**Evidence:**
- Screenshot: Limit field shows "100"
- Network: POST with `"limit": 100`
- Task log: "Processing up to 100 items"

### 4.2 Leave Limit Empty
**User Action:** Don't enter any limit value
- Leave field blank
- Submit form

**Expected Outcome:**
- Backend treats as unlimited/no limit
- Task processes all matching items
- No validation error

**Evidence:**
- Network: POST with `"limit": null` or field omitted
- Task log: "Processing all items (no limit)"

### 4.3 Enter Zero Limit (Edge Case)
**User Action:** Enter "0" in limit field
- Type "0"
- Submit form

**Expected Outcome:**
- Validation error: "Limit must be greater than 0"
- Or: Backend treats as unlimited
- Submit blocked or warning shown

**Evidence:**
- Console: Validation error
- Screenshot: Error message "Invalid limit value"
- Network: 400 Bad Request or field rejected

### 4.4 Enter Negative Limit (Edge Case)
**User Action:** Enter negative number
- Type "-10"
- Submit form

**Expected Outcome:**
- Validation error: "Limit must be positive"
- Frontend prevents negative input (HTML5 min="1")
- Backend rejects if bypassed

**Evidence:**
- Screenshot: Input field rejects negative
- Console: Validation error
- Network: 400 Bad Request if sent

### 4.5 Enter Non-Numeric Limit (Edge Case)
**User Action:** Enter text instead of number
- Type "abc"
- Type "10.5" (decimal)
- Type "1e5" (scientific notation)

**Expected Outcome:**
- HTML5 number input prevents non-numeric
- If bypassed: validation error "Limit must be an integer"
- Decimal rounds or rejects

**Evidence:**
- Screenshot: Input field shows validation state
- Console: Type validation error
- Network: 400 Bad Request

### 4.6 Enter Very Large Limit (Edge Case)
**User Action:** Enter extremely large number
- Type "999999999"
- Type "2147483647" (max int32)

**Expected Outcome:**
- Backend accepts or caps at reasonable maximum
- Warning: "Large limit may cause performance issues"
- Task executes but may timeout

**Evidence:**
- Network: POST with large limit value
- Backend log: Warning about large limit
- Task status: Running or timeout error

---

## 5. FORM VALIDATION

### 5.1 Empty Required Fields
**User Action:** Submit form with missing required data
- Leave task name empty
- Leave target (dbnum/refno) empty
- Click Submit

**Expected Outcome:**
- Frontend validation prevents submission
- Error messages highlight missing fields
- Submit button disabled or form shows errors

**Evidence:**
- Screenshot: Red borders on empty required fields
- Console: "Please fill all required fields"
- Network: No POST request sent

### 5.2 Duplicate Task Name
**User Action:** Enter task name that already exists
- Type name of existing task
- Submit form

**Expected Outcome:**
- Backend checks for duplicate names
- Returns error: "Task name already exists"
- Suggests alternatives with timestamp/suffix

**Evidence:**
- Network: 400 Bad Request with `"error_type": "duplicate_name"`
- Response includes suggestions: `["Task Name - 20260312_115005", "Task Name (2)"]`
- Screenshot: Error message with suggestions

### 5.3 Invalid dbnum Format
**User Action:** Enter malformed dbnum
- Type "abc7997"
- Type "79.97"
- Type "7997xyz"

**Expected Outcome:**
- Validation error: "dbnum must be a valid integer"
- Input field shows error state
- Submit blocked

**Evidence:**
- Console: Type validation error
- Screenshot: Error message below field
- Network: No POST or 400 Bad Request

### 5.4 Invalid refno Format
**User Action:** Enter malformed refno
- Type "1//2" (double slash)
- Type "/123" (leading slash)
- Type "123/" (trailing slash)
- Type "a/b/c" (non-numeric)

**Expected Outcome:**
- Validation error: "Invalid refno format"
- Expected format shown: "123 or 1/456"
- Submit blocked

**Evidence:**
- Console: Format validation error
- Screenshot: Error tooltip/message
- Network: 400 Bad Request if sent

---

## 6. SUBMISSION & NAVIGATION

### 6.1 Successful Task Creation
**User Action:** Fill all fields correctly and submit
- Task name: "Test Model Generation"
- Type: DataGeneration
- dbnum: 7997
- nouns: ["BRAN", "HANG"]
- limit: 100
- Click Submit

**Expected Outcome:**
- Success message: "Task created successfully"
- Task ID returned: "uuid-string"
- Redirect to task detail page or task list
- Task appears in active tasks list

**Evidence:**
- Network: POST `/api/tasks/create` returns 200 with `{"success": true, "task_id": "..."}`
- Screenshot: Success notification
- Navigation: Redirected to `/tasks/{task_id}` or `/tasks`
- Task list: New task visible with "Pending" status

### 6.2 Server Error Handling
**User Action:** Submit when backend is unavailable
- Fill form correctly
- Backend returns 500 error
- Network timeout

**Expected Outcome:**
- Error message: "Failed to create task. Please try again."
- Form data preserved (not cleared)
- Retry button available

**Evidence:**
- Network: 500 Internal Server Error or timeout
- Console: Error logged
- Screenshot: Error notification with retry option

### 6.3 Cancel/Navigate Away
**User Action:** Start filling form then cancel
- Fill some fields
- Click Cancel button
- Navigate to different page

**Expected Outcome:**
- Confirmation dialog: "Discard unsaved changes?"
- If confirmed: Navigate away, data lost
- If cancelled: Stay on form

**Evidence:**
- Screenshot: Confirmation dialog
- Console: Navigation prevented
- Form data: Cleared after confirmation

---

## 7. EDGE CASES & SPECIAL SCENARIOS

### 7.1 Non-Existent dbnum
**User Action:** Enter valid format but non-existent dbnum
- Type "99999" (doesn't exist in database)
- Submit form

**Expected Outcome:**
- Task created successfully (validation passes)
- Task execution fails with error: "Database 99999 not found"
- Task status: Failed
- Error details in task logs

**Evidence:**
- Network: POST returns 200 (creation succeeds)
- Task detail: Status = "Failed"
- Task logs: "Error: Database 99999 not found"

### 7.2 Non-Existent refno
**User Action:** Enter valid format but non-existent refno
- Type "999/999999"
- Submit form

**Expected Outcome:**
- Task created successfully
- Task execution fails: "Reference number 999/999999 not found"
- Task status: Failed

**Evidence:**
- Network: POST returns 200
- Task detail: Status = "Failed"
- Task logs: "Error: refno not found"

### 7.3 Concurrent Task Creation
**User Action:** Create multiple tasks rapidly
- Submit form
- Immediately submit again with different data
- Create 5 tasks in quick succession

**Expected Outcome:**
- All tasks created with unique IDs
- No race conditions or duplicate IDs
- All tasks appear in task list

**Evidence:**
- Network: Multiple POST requests, all return unique task_ids
- Task list: All tasks visible
- Database: All tasks persisted correctly

### 7.4 Special Characters in Task Name
**User Action:** Enter task name with special characters
- Type "Test Task #1 (2024) - [URGENT]"
- Type "任务测试 中文名称"
- Type "Task with emoji 🚀"

**Expected Outcome:**
- Characters accepted (or sanitized)
- Task created successfully
- Name displayed correctly in UI

**Evidence:**
- Network: POST with encoded name
- Task list: Name displayed correctly
- Database: Name stored correctly

### 7.5 Browser Refresh During Creation
**User Action:** Refresh page while task is being created
- Click Submit
- Immediately refresh browser (F5)

**Expected Outcome:**
- Task creation completes on backend
- After refresh: Task appears in list
- Or: Task creation cancelled, no orphaned task

**Evidence:**
- Network: POST request completes or cancelled
- Task list: Task present or absent (consistent state)
- No duplicate tasks created

### 7.6 Session Timeout
**User Action:** Leave form open for extended period
- Fill form
- Wait 30+ minutes
- Submit form

**Expected Outcome:**
- Session validation on submit
- If expired: Redirect to login or show error
- If valid: Task created successfully

**Evidence:**
- Network: 401 Unauthorized if session expired
- Screenshot: Session timeout message
- Or: Task created successfully

---

## 8. VALIDATION CONTRACT ASSERTIONS

Based on the above interactions, the validation contract should assert:

### Frontend Validations
1. ✅ Task name is required and non-empty
2. ✅ Task type is selected
3. ✅ Either dbnum OR refno is provided (mutually exclusive)
4. ✅ dbnum is positive integer if provided
5. ✅ refno matches format `\d+(/\d+)*` if provided
6. ✅ Limit is positive integer or empty
7. ✅ Nouns array contains only valid noun types

### Backend Validations
1. ✅ Task name uniqueness check
2. ✅ dbnum/refno mutual exclusivity
3. ✅ dbnum format and range validation
4. ✅ refno format validation
5. ✅ Noun types validation against known list
6. ✅ Limit range validation (> 0 if provided)
7. ✅ Task type is valid enum value

### Execution Validations
1. ✅ dbnum exists in database (runtime check)
2. ✅ refno exists in database (runtime check)
3. ✅ Selected nouns have data to process
4. ✅ Limit applied correctly during processing

### Response Validations
1. ✅ Success response includes task_id
2. ✅ Error responses include error_type and details
3. ✅ Duplicate name errors include suggestions
4. ✅ Task status updates correctly (Pending → Running → Completed/Failed)

---

## Summary

This enumeration covers **50+ distinct user interactions** across:
- Basic flow (3 interactions)
- Target selection (4 scenarios + 4 edge cases)
- Noun filtering (5 scenarios)
- Limit parameter (6 scenarios)
- Form validation (4 scenarios)
- Submission (3 scenarios)
- Edge cases (6 scenarios)

Each interaction specifies:
- **Action**: What the user does
- **Outcome**: What they expect to see
- **Evidence**: How to verify it works (screenshot, console, network, logs)

This provides a comprehensive foundation for validation contract implementation and test case generation.
