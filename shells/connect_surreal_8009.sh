#!/bin/bash

# Kill any process using port 8009
lsof -ti:8009 | xargs -r kill -9 2>/dev/null || true

surreal sql --endpoint http://127.0.0.1:8009 --user root --pass root --ns 1516 --db AvevaMarineSample
