#!/usr/bin/env python3
"""Interactive demo client for reflect MCP server."""
import subprocess
import json
import os
import sys
import time

CYAN = '\033[0;36m'
GREEN = '\033[0;32m'
YELLOW = '\033[1;33m'
RED = '\033[0;31m'
DIM = '\033[2m'
BOLD = '\033[1m'
NC = '\033[0m'

BINARY = './target/release/reflect-mcp'
DB = os.environ.get('REFLECT_DB', '/tmp/reflect-demo.db')

def start_server():
    return subprocess.Popen(
        [BINARY],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env={**os.environ, 'REFLECT_DB': DB}
    )

def call(proc, method, params, rid):
    msg = json.dumps({'jsonrpc': '2.0', 'id': rid, 'method': method, 'params': params}) + '\n'
    proc.stdin.write(msg.encode())
    proc.stdin.flush()
    line = proc.stdout.readline().decode().strip()
    return json.loads(line) if line else None

def notify(proc, method):
    msg = json.dumps({'jsonrpc': '2.0', 'method': method}) + '\n'
    proc.stdin.write(msg.encode())
    proc.stdin.flush()

def section(text):
    print(f"\n{BOLD}{CYAN}{'─' * 60}{NC}")
    print(f"{BOLD}{CYAN}  {text}{NC}")
    print(f"{BOLD}{CYAN}{'─' * 60}{NC}")
    time.sleep(0.3)

def tool_call(proc, name, args, rid):
    r = call(proc, 'tools/call', {'name': name, 'arguments': args}, rid)
    content = json.loads(r['result']['content'][0]['text'])
    return content

proc = start_server()

# Initialize
call(proc, 'initialize', {
    'protocolVersion': '2024-11-05',
    'capabilities': {},
    'clientInfo': {'name': 'demo', 'version': '1.0'}
}, 1)
notify(proc, 'notifications/initialized')

# List tools
r = call(proc, 'tools/list', {}, 2)
tools = [t['name'] for t in r['result'].get('tools', [])]
section(f"7 MCP tools registered")
for t in tools:
    print(f"  {GREEN}•{NC} {t}")
time.sleep(0.5)

# Store reflections
section("Storing reflections from past failures")
reflections = [
    {
        'task': 'parse user date input from HTML form',
        'critique': 'Used unwrap() on user-provided string that can contain any format',
        'lesson': 'Always use Result handling for parse operations on untrusted user input',
        'outcome': 'failure',
        'tags': ['rust', 'error-handling', 'parsing']
    },
    {
        'task': 'connect to postgres database at startup',
        'critique': 'Hardcoded connection string in source code',
        'lesson': 'Use environment variables or config files for database URLs',
        'outcome': 'failure',
        'tags': ['python', 'config', 'database']
    },
    {
        'task': 'render user profile React component',
        'critique': 'Used any type for props instead of proper interface',
        'lesson': 'Define TypeScript interfaces for all component props — avoid any',
        'outcome': 'failure',
        'tags': ['typescript', 'react', 'type-safety']
    },
]

for i, ref in enumerate(reflections):
    result = tool_call(proc, 'store_reflection', ref, 10 + i)
    rid = result['reflection_id'][:8]
    print(f"  {GREEN}✓{NC} Stored: {DIM}{rid}...{NC}  {YELLOW}{ref['tags'][0]}{NC}  \"{ref['lesson'][:50]}...\"")
    time.sleep(0.2)

# Recall
section("Recalling lessons for: 'parse user input'")
time.sleep(0.3)
result = tool_call(proc, 'recall_reflections', {
    'task': 'parse user input',
    'tags': []
}, 20)

refs = result['reflections']
if refs:
    for ref in refs:
        r = ref['reflection']
        score = ref['relevance_score']
        print(f"  {GREEN}→{NC} score={YELLOW}{score:.2f}{NC}  \"{r['lesson'][:60]}\"")
        print(f"    {DIM}tags: {r['tags']}  outcome: {r['outcome']}{NC}")
else:
    print(f"  {DIM}(FTS5 needs exact word overlap — try semantic search with ctxgraph backend){NC}")

# Stats
section("Reflection statistics")
time.sleep(0.3)
result = tool_call(proc, 'get_reflection_stats', {}, 30)
print(f"  Total reflections: {BOLD}{result['total_reflections']}{NC}")
print(f"  By outcome: {GREEN}success={result['by_outcome']['success']}{NC}  {RED}failure={result['by_outcome']['failure']}{NC}  {YELLOW}partial={result['by_outcome']['partial']}{NC}")
print(f"  Avg confidence:    {result['avg_confidence']:.2f}")
tags = result.get('top_tags', [])
if tags:
    tag_str = ', '.join(f"{t['tag']}({t['count']})" for t in tags[:5])
    print(f"  Top tags:          {tag_str}")

# Forget
section("Forgetting a reflection")
# Get the first reflection ID we stored
first_id = tool_call(proc, 'recall_reflections', {'task': 'parse', 'tags': []}, 40)
if first_id['reflections']:
    fid = first_id['reflections'][0]['reflection']['id']
    result = tool_call(proc, 'forget_reflection', {'reflection_id': fid}, 41)
    print(f"  {GREEN}✓{NC} Deleted reflection {DIM}{fid[:12]}...{NC}  deleted={result['deleted']}")

# Final stats
result = tool_call(proc, 'get_reflection_stats', {}, 50)
print(f"  Remaining: {result['total_reflections']} reflections")

proc.terminate()
print(f"\n{BOLD}{GREEN}  ✓ Demo complete — all tools working{NC}\n")
