#!/usr/bin/env bash
# Log cruncher with one subtle bug. Hunt it down:
#
#   trapdoor -r -b 'pipeline.sh:36' examples/pipeline.sh
#   (tdb) p requests errors total_bytes
#
# Expected average is 679 — the script prints 776. Why?
set -u

server_log() {
    cat <<'EOF'
127.0.0.1 404 234
192.168.1.1 500 1024
::1 200 345
10.0.0.1 503 512
172.16.0.1 200 678
fe80::1 504 765
192.168.0.1 403 890
10.0.0.2 500 987
EOF
}

requests=0
errors=0
total_bytes=0

while read -r ip status bytes; do
    requests=$((requests + 1))
    if (( status >= 500 )); then
        errors=$((errors + 1))
    fi
    total_bytes=$((total_bytes + bytes))
done < <(server_log)

# Off-by-one: "skip the header line" — but this log has no header.
average=$((total_bytes / (requests - 1)))

echo "requests=$requests errors=$errors total_bytes=$total_bytes avg_bytes=$average"
