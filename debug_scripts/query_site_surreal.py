import json
import pathlib
import sys
import tomllib

import requests


config_path = pathlib.Path(sys.argv[1])
query = sys.argv[2]
config = tomllib.loads(config_path.read_text(encoding="utf-8"))
surreal = config["surrealdb"]
response = requests.post(
    f"http://{surreal['ip']}:{surreal['port']}/sql",
    headers={
        "surreal-ns": str(config.get("namespace", 1)),
        "surreal-db": config.get("database", "AvevaPlantSample"),
        "Accept": "application/json",
    },
    auth=(surreal["user"], surreal["password"]),
    data=query,
    timeout=30,
)
response.raise_for_status()
print(json.dumps(response.json(), ensure_ascii=False, indent=2))
