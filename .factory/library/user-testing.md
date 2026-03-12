# User Testing

Testing surface: tools, URLs, setup steps, isolation notes, known quirks.

**What belongs here:** How to manually test the application, what tools to use, test accounts, known issues.

---

## Testing Surface

**Backend API:**
- Base URL: http://localhost:3100
- Key endpoints: /api/tasks, /api/status, /api/config
- Test with: curl, Postman, or browser DevTools

**Frontend UI:**
- Base URL: http://localhost:3101
- Entry point: Project selection → Task creation
- Test with: agent-browser, manual browser testing

## Testing Tools

**agent-browser:**
- Available at: ~/.factory/bin/agent-browser
- Usage: Navigate to http://localhost:3101, interact with UI
- Can take screenshots, extract page structure

**curl:**
- Test API endpoints directly
- Example: `curl http://localhost:3100/api/status`

## Test Workflow

1. **Create task**: Open http://localhost:3101, select AvevaMarineSample project, click task creation
2. **Fill form**: Enter name, select DataGeneration, add dbnum=7997, nouns=["BRAN"], limit=10
3. **Monitor**: Watch progress bar update in real-time
4. **Preview**: Click preview button when complete, verify model loads

## Known Quirks

- API status may show `database_connected: false` even when connected (check logs)
- WebSocket updates may have 1-2 second delay (normal)
- Frontend dev server may need restart after major changes
