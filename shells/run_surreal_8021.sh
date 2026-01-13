lsof -ti:8021 | xargs -r kill -9 2>/dev/null || true
surreal start --user root --pass root --bind 0.0.0.0:8021 rocksdb://ams-8021.db
