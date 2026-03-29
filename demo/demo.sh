#!/usr/bin/env bash
# reflect demo — run with: asciinema rec --command ./demo/demo.sh demo.cast
set -e

# Colors
CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
DIM='\033[2m'
BOLD='\033[1m'
NC='\033[0m'

type_slow() {
    local text="$1"
    local delay="${2:-0.04}"
    for (( i=0; i<${#text}; i++ )); do
        printf "%s" "${text:$i:1}"
        sleep "$delay"
    done
    echo
}

run_cmd() {
    printf "${GREEN}\$ ${NC}"
    type_slow "$1" 0.03
    sleep 0.3
    eval "$1"
    sleep 0.8
}

comment() {
    printf "\n${CYAN}# %s${NC}\n" "$1"
    sleep 0.6
}

BINARY="./target/release/reflect-mcp"
export REFLECT_DB="/tmp/reflect-demo.db"
rm -f "$REFLECT_DB"

clear
printf "${BOLD}${CYAN}"
cat << 'BANNER'
         __ _           _
  _ _ __|  | |___  __  | |_
 | '_/ -_| |  _/ -_) _||  _|
 |_| \___|_|\__\___\__| \__|

  Self-correction engine for AI coding agents
  Implements: Reflexion (Shinn et al., NeurIPS 2023)

BANNER
printf "${NC}"
sleep 1.5

comment "reflect is a Rust MCP server that turns agent failures into lessons"
comment "Let's see it in action"
sleep 0.5

comment "Step 1: Check the binary"
run_cmd "ls -lh $BINARY"

comment "Step 2: Start the server and list available tools"
run_cmd "echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"demo\",\"version\":\"1.0\"}}}' | $BINARY 2>/dev/null | python3 -c \"import sys,json; r=json.load(sys.stdin)['result']; print(f'Server: {r[\\\"serverInfo\\\"][\\\"name\\\"]} v{r[\\\"serverInfo\\\"][\\\"protocolVersion\\\"]}')\" "

comment "Step 3: Store a reflection — agent learned from a failure"
cat << 'EOF' > /tmp/reflect-demo-input.jsonl
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"demo","version":"1.0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"store_reflection","arguments":{"task":"parse user date input from form","critique":"Used unwrap() on user-provided string that can contain any format","lesson":"Always use Result handling for parse operations on untrusted user input — never unwrap","outcome":"failure","tags":["rust","error-handling","parsing"]}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"store_reflection","arguments":{"task":"connect to postgres database","critique":"Hardcoded connection string in source code","lesson":"Use environment variables or config files for database connection strings","outcome":"failure","tags":["python","config","database"]}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"store_reflection","arguments":{"task":"render user profile component","critique":"Used any type for props instead of proper interface","lesson":"Define TypeScript interfaces for all component props — avoid any","outcome":"failure","tags":["typescript","react","type-safety"]}}}
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"recall_reflections","arguments":{"task":"parse a date string from API response","tags":["rust"]}}}
{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"get_reflection_stats","arguments":{}}}
EOF

run_cmd "python3 demo/demo_client.py"

comment "That's reflect — persistent memory for AI agents across sessions"
printf "\n${BOLD}${YELLOW}  github.com/rohansx/reflect${NC}\n\n"
sleep 2
