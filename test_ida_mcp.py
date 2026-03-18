import subprocess
import json
import sys

def run_mcp():
    proc = subprocess.Popen(
        [sys.executable, "/opt/homebrew/lib/python3.14/site-packages/ida_pro_mcp/server.py", "--ida-rpc", "http://127.0.0.1:8745"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    
    # Send init
    init_req = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test-client", "version": "1.0.0"}
        }
    }
    proc.stdin.write(json.dumps(init_req) + "\n")
    proc.stdin.flush()
    print("Init response:", proc.stdout.readline())
    
    # Send initialized notification
    init_notif = {
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }
    proc.stdin.write(json.dumps(init_notif) + "\n")
    proc.stdin.flush()
    
    # List tools
    list_req = {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    }
    proc.stdin.write(json.dumps(list_req) + "\n")
    proc.stdin.flush()
    print("Tools list:", proc.stdout.readline())
    proc.kill()

if __name__ == "__main__":
    run_mcp()
