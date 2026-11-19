"""Synthetic stdio peers for Codex App Server and DSH ACP. No network or files."""
import json
import sys

platform = sys.argv[1]
session = 'synthetic-session'
active_prompt = None


def emit(value):
    print(json.dumps(value), flush=True)


for line in sys.stdin:
    message = json.loads(line)
    method = message.get('method')
    params = message.get('params', {})
    if method == 'initialize':
        emit({'id': message['id'], 'result': {'protocolVersion': 1, 'agentCapabilities': {'loadSession': True}}})
    elif method in ('thread/start', 'thread/resume', 'session/new', 'session/load'):
        assert method.startswith('thread/') == (platform == 'codex')
        if platform == 'codex':
            assert params['config']['mcp_servers']['drifting']['args'] == ['--mcp', '--connection', 'synthetic.json']
            result = {'thread': {'id': session}}
        else:
            assert params['mcpServers'][0]['args'] == ['--mcp', '--connection', 'synthetic.json']
            result = {'sessionId': session}
        emit({'id': message['id'], 'result': result})
    elif method in ('turn/start', 'session/prompt'):
        active_prompt = message['id']
        assert method.startswith('turn/') == (platform == 'codex')
        if platform == 'codex':
            emit({'method': 'turn/started', 'params': {'turn': {'id': 'synthetic-turn', 'status': 'inProgress'}}})
            for text in ['synthetic ', 'stream']:
                emit({'method': 'item/agentMessage/delta', 'params': {'delta': text}})
            emit({'id': 99, 'method': 'mcpServer/elicitation/request', 'params': {'serverName': 'drifting', 'mode': 'form', 'message': 'Allow synthetic MCP write?', 'requestedSchema': {'type': 'object', 'properties': {}}}})
        else:
            for text in ['synthetic ', 'stream']:
                emit({'method': 'session/update', 'params': {'sessionId': session, 'update': {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': text}}}})
            emit({'id': 99, 'method': 'session/request_permission', 'params': {'sessionId': session, 'options': []}})
    elif message.get('id') == 99 and 'result' in message:
        if platform == 'codex':
            emit({'method': 'turn/completed', 'params': {'turn': {'id': 'synthetic-turn', 'status': 'completed'}}})
        else:
            emit({'id': active_prompt, 'result': {'stopReason': 'end_turn'}})
    elif method in ('turn/interrupt', 'session/cancel'):
        if platform == 'codex':
            emit({'method': 'turn/completed', 'params': {'turn': {'id': 'synthetic-turn', 'status': 'interrupted'}}})
        else:
            emit({'id': active_prompt, 'result': {'stopReason': 'cancelled'}})
    elif method == 'fixture/disconnect':
        break
