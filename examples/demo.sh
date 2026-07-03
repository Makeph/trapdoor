#!/usr/bin/env bash
# Toy pipeline: step through it, break inside the loop, rewrite variables live.
set -u

greet() {
    local who=$1
    local msg="hello, $who"
    echo "$msg"
}

count=0
for fruit in apple banana cherry; do
    count=$((count + 1))
    greet "$fruit"
done

total=$((count * 14))
echo "processed $count fruits, total=$total"
