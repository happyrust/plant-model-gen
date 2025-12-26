lsof -ti:8020 | xargs -r kill -9 2>/dev/null || true
surreal start --user root --pass root --bind 0.0.0.0:8020 rocksdb://ams-8020.db
