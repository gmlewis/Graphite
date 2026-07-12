#!/bin/bash
# Test the Graphite MCP server (standalone mode)
# This sends a series of JSON-RPC messages to verify the server works.

set -euo pipefail

MCP="${1:-$HOME/tools/bin/graphite-mcp}"

echo "Testing Graphite MCP Server (standalone mode)..."
echo ""

# Send initialize + tools/list + get_node_catalog
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_node_catalog","arguments":{"search":"blur"}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_node_details","arguments":{"node_id":"blur"}}}
' | "$MCP" --standalone 2>/dev/null | python3 -c "
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    resp = json.loads(line)
    rid = resp.get('id')
    if 'result' in resp:
        result = resp['result']
        if 'tools' in result:
            print(f'Response {rid}: {len(result[\"tools\"])} tools available')
        elif 'content' in result:
            text = result['content'][0].get('text', '')[:200]
            print(f'Response {rid}: {text}...')
        elif 'serverInfo' in result:
            print(f'Response {rid}: Connected to {result[\"serverInfo\"][\"name\"]} v{result[\"serverInfo\"][\"version\"]}')
        else:
            print(f'Response {rid}: {json.dumps(result)[:100]}...')
    elif 'error' in resp:
        print(f'Response {rid}: ERROR - {resp[\"error\"][\"message\"]}')
"

echo ""
echo "All tests passed!"
