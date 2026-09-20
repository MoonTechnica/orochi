"""Deterministic ACP v1 fixture. Never contacts an LLM or executes task text."""
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import time
import uuid

behavior = os.environ.get("MOCK_BEHAVIOR", "success")
log_path = os.environ.get("MOCK_LOG")
adviser_turns = 0
adviser_ids = []
models = os.environ.get("MOCK_MODELS", "sol-test,astra-test").split(",")
model = models[0]
reasoning = "medium"
mode = "ask"
session = "mock-" + uuid.uuid4().hex
root = None


mcp_servers = []
if behavior == "permission_persist":
    # Unbuffered, so waiting on the descriptor sees a line that has already arrived.
    sys.stdin = open(0, "rb", buffering=0)


def mailbox_tools():
    """Minimal MCP client for the orochi-mailbox server attached through session/new."""
    server = next(s for s in mcp_servers if s["name"] == "orochi-mailbox")
    env = dict(os.environ, **{e["name"]: e["value"] for e in server.get("env", [])})
    proc = subprocess.Popen([server["command"], *server["args"]], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, text=True, env=env)
    counter = [0]

    def rpc(method, params):
        counter[0] += 1
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": counter[0], "method": method, "params": params}) + "\n")
        proc.stdin.flush()
        return json.loads(proc.stdout.readline())

    rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "mock", "version": "1"}})
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
    names = [t["name"] for t in rpc("tools/list", {})["result"]["tools"]]
    assert names == ["list_peers", "send_message", "read_messages", "set_status"], names

    def tool(name, **arguments):
        reply = rpc("tools/call", {"name": name, "arguments": arguments})["result"]
        assert not reply["isError"], reply
        return json.loads(reply["content"][0]["text"])
    return proc, tool


def mailbox_chat(request):
    try:
        converse(request)
    except BaseException:
        # Surface fixture failures to the test; agent stderr is not shown by Orochi.
        record_failure()
        raise


def record_failure():
    if not log_path:
        return
    import traceback
    with open(log_path + ".err", "a") as out:
        out.write(f"--- pid {os.getpid()} at {time.time():.1f}\n" + traceback.format_exc())


def converse(request):
    text = request["params"]["prompt"][0]["text"]
    assert "orochi-mailbox" in text, "missing coordination note"
    proc, tool = mailbox_tools()
    role, other = os.environ["MOCK_MAILBOX_ROLE"], os.environ["MOCK_MAILBOX_PEER"]
    deadline = time.time() + 20
    while not any(p["name"] == other for p in tool("list_peers")["peers"]):
        assert time.time() < deadline, "peer never appeared"
        time.sleep(0.1)
    if role == "sender":
        tool("set_status", status="sending a greeting")
        tool("send_message", to=other, body="hello from sender")
        got = tool("read_messages", wait_seconds=20)["messages"]
        assert got, "no reply"
        (root / "reply.txt").write_text(got[0]["body"])
    else:
        got = tool("read_messages", wait_seconds=20)["messages"]
        assert got and got[0]["body"] == "hello from sender", got
        tool("send_message", to=got[0]["from"], body="ack: " + got[0]["body"])
        (root / "received.txt").write_text(got[0]["body"])
    proc.terminate()


def seats(request):
    """Two seats of one Orochi process: the lead asks, the read-only seat answers."""
    text = request["params"]["prompt"][0]["text"]
    if "you own every change" not in text.lower() and "every write tool is refused" not in text:
        return  # A step that runs on its own (design, review): nothing to coordinate.
    proc, tool = mailbox_tools()
    me = next(p["name"] for p in tool("list_peers")["peers"] if p["you"])
    if "you own every change" in text.lower():
        wanted = int(os.environ.get("MOCK_SEATS", "1"))
        deadline, others = time.time() + 40, []
        while len(others) < wanted and time.time() < deadline:
            others = [p["name"] for p in tool("list_peers")["peers"] if not p["you"]]
            time.sleep(0.1)
        assert len(others) >= wanted, f"{me}: only {others} joined, wanted {wanted}"
        tool("send_message", to="all", body="anything to know before I touch the parser?")
        answers = {}
        while len(answers) < wanted and time.time() < deadline:
            for got in tool("read_messages", wait_seconds=20)["messages"]:
                answers[got["from"]] = got["body"]
        assert len(answers) >= wanted, f"only {list(answers)} answered"
        (root / "advice.txt").write_text(
            "\n".join(f"{name}: {body}" for name, body in sorted(answers.items())))
    else:
        got = tool("read_messages", wait_seconds=30)["messages"]
        assert got, "the lead never asked anything"
        tool("send_message", to=got[0]["from"], body="watch the error path, it is unhandled")
        (root / "asked.txt").write_text(me)
        # A read-only seat's write tools are refused before the user is ever asked.
        send({"id": "permission-1", "method": "session/request_permission", "params": {
            "sessionId": session, "toolCall": {"toolCallId": "call-1", "title": "Patch the parser", "kind": "edit"},
            "options": [{"optionId": "deny", "name": "Deny", "kind": "reject_once"},
                        {"optionId": "allow", "name": "Allow", "kind": "allow_once"}]}})
        answer = json.loads(sys.stdin.readline())
        (root / "refused.txt").write_text(json.dumps(answer.get("result", {})))
        if os.environ.get("MOCK_MID_REPLY"):
            # Speak up while the lead is answering, not before: the lead writes advice.txt
            # just before it starts streaming its reply.
            deadline = time.time() + 30
            while not (root / "advice.txt").exists() and time.time() < deadline:
                time.sleep(0.05)
            time.sleep(0.8)
            tool("send_message", to=got[0]["from"], body="one more thing: the retry has no backoff")
    proc.terminate()


def send(value):
    print(json.dumps(dict(jsonrpc="2.0", **value)), flush=True)


def result(request, value):
    send({"id": request["id"], "result": value})


def options():
    levels = ["high", "xhigh"] if "astra" in model else ["low", "medium"]
    return [
        {"id": "engine", "category": "model", "type": "select", "name": "Model", "currentValue": model,
         "options": [{"group": "test-models", "name": "Test models", "options": [{"value": m, "name": m} for m in models]}]},
        {"id": "effort-control", "category": "thought_level", "type": "select", "name": "Effort", "currentValue": reasoning,
         "options": [{"value": v, "name": v} for v in levels]},
        {"id": "mode", "category": "mode", "type": "select", "name": "Mode", "currentValue": mode,
         "options": [{"value": "ask", "name": "Ask"}, {"value": "plan", "name": "Plan"}]},
    ]


def finish(request):
    # A console turn outside a collaboration (a design step, a follow-up) has no role.
    if behavior == "session_collaboration" and re.search(r'Role: (\w+)\.', request["params"]["prompt"][0]["text"]):
        prompt = request["params"]["prompt"][0]["text"]
        role = re.search(r'Role: (\w+)\.', prompt)[1].lower()
        fail_role = os.environ.get("MOCK_FAIL_ROLE")
        fail_stage = os.environ.get("MOCK_FAIL_STAGE")
        stage = re.search(r'stage (\d+)\.', prompt)[1]
        fail_model = os.environ.get("MOCK_FAIL_MODEL")
        fail_part = os.environ.get("MOCK_FAIL_PART")
        participant = re.search(r'You are participant ([\w-]+) in', prompt)[1]
        if fail_role == role and (fail_stage is None or fail_stage == stage) and (fail_model is None or fail_model == model) \
                and (fail_part is None or fail_part == participant):
            send({"method": "session/update", "params": {"sessionId": session, "update": {
                "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "CHECKPOINT_NOTE: preserve the chosen plan; unresolved issue remains"}}}})
            if role in ("implementer", "integrator"):
                (root / "partial.txt").write_text("preserved partial implementation")
            failure = os.environ.get("MOCK_FAILURE_KIND", "rate")
            if failure == "disconnect": sys.exit(2)
            if failure == "hang":
                (root / "ready.pid").write_text(str(os.getpid()))
                time.sleep(60)
            elif failure == "cancel": result(request, {"stopReason": "cancelled"})
            else:
                send({"id": request["id"], "error": {"code": 402 if failure == "credit" else 429,
                    "message": "credit balance is too low" if failure == "credit" else "rate limit exceeded",
                    "data": {"resetAt": int(time.time())+3600, "scope": "model" if failure == "model_rate" else "agent"}}})
            return
        if os.environ.get("MOCK_EXPECT_HANDOFF") == role:
            assert "CHECKPOINT_NOTE" in prompt, "missing partial handoff"
            if role in ("implementer", "integrator"):
                assert (root / "partial.txt").read_text() == "preserved partial implementation"
        parallel = os.environ.get("MOCK_PARALLEL")
        parts = os.environ.get("MOCK_PARTS")
        discussion = os.environ.get("MOCK_DISCUSSION")
        messages = []
        text = "Implementation ready"
        if "Resolve the conflicts between parts" in prompt:
            shared = root / "shared.txt"
            assert "<<<<<<< " in shared.read_text()
            if not os.environ.get("MOCK_KEEP_MARKERS"):
                shared.write_text("alpha+beta\n")
            text = "Conflicts between parts resolved"
        elif "Resolve these merge conflicts" in prompt:
            seed = root / "seed.txt"
            assert "<<<<<<< working-tree" in seed.read_text()
            seed.write_text(os.environ["MOCK_RESOLVED"])
            once = os.environ.get("MOCK_EDIT_DURING_RESOLUTION")
            if once and not Path(once + ".done").exists():
                Path(once + ".done").write_text("")
                Path(once).write_text("line1\nuser3\nline3\n")
            text = "Conflicts resolved"
        elif "Discussion round" in prompt:
            if role == "implementer":
                assert "Please add input validation" in prompt
                (root / "validation.txt").write_text("validated")
                messages = [{"to": "reviewer", "body": "Added validation"}]
                text = "Validation added"
            else:
                assert "Added validation" in prompt
                text = "Acknowledged"
        elif role == "coordinator":
            state = {"plan": ["Implement, review and integrate"], "decisions": ["Check edge cases"], "open_questions": ["Verify result"], "next_step": "Follow the next scheduled stage"}
            proposal = os.environ.get("MOCK_COORDINATOR_PARTS" if stage == "0" else "MOCK_COORDINATOR_LATER_PARTS")
            if proposal:
                state["parts"] = json.loads(proposal)
            text = json.dumps(state)
            if os.environ.get("MOCK_COORDINATOR_PREFIX"):
                text = "Warning: local CLI notice\n\n```json\n" + text + "\n```"
        elif role == "reviewer":
            if parts:
                for part in filter(None, os.environ.get("MOCK_EXPECT_PARTS", "").split(",")):
                    assert (root / part / "done.txt").exists(), f"the reviewer is missing {part}"
            elif parallel:
                assert (root / "alice.txt").exists() and (root / "bob.txt").exists()
            else:
                assert (root / "completed.txt").read_text() == "implementation"
                (root / "completed.txt").write_text("review edit must not leak")
            if discussion:
                messages = [{"to": "AUTHOR", "body": "Please add input validation"}, {"to": "nobody", "body": "lost"}]
            text = "REVIEW_FINDING: integrate the confirmed fix"
        elif "Integrate the implementation" in prompt:
            if parts:
                pass
            elif parallel:
                shared = root / "shared.txt"
                assert "<<<<<<< " in shared.read_text() and "shared.txt" in prompt
                if not os.environ.get("MOCK_KEEP_MARKERS"):
                    shared.write_text("alice+bob\n")
            else:
                assert (root / "completed.txt").read_text() == "implementation"
                assert "REVIEW_FINDING" in prompt
                if discussion:
                    assert "Added validation" in prompt and (root / "validation.txt").exists()
            (root / "completed.txt").write_text("integrated")
            text = "Review finding addressed"
        elif parts and re.search(r'Your part: ([a-z0-9-]+)', prompt):
            part = re.search(r'Your part: ([a-z0-9-]+)', prompt)[1]
            together = os.environ.get("MOCK_RENDEZVOUS_PARTS", "").split(",")
            if part in together:
                meeting = Path(os.environ["MOCK_RENDEZVOUS"])
                (meeting / part).write_text("started")
                deadline = time.time() + 25
                while len(list(meeting.iterdir())) < len(together):
                    assert time.time() < deadline, "the parts of one wave did not run concurrently"
                    time.sleep(0.05)
            needs = dict(item.split(":") for item in os.environ.get("MOCK_PART_NEEDS", "").split(";") if item)
            for need in filter(None, needs.get(part, "").split("/")):
                assert (root / need / "done.txt").read_text() == need, f"{part} started without {need}"
            shared = root / "shared.txt"
            if part in needs and shared.exists():
                assert "<<<<<<< " not in shared.read_text(), f"{part} started on an unresolved merge"
            (root / part).mkdir(exist_ok=True)
            (root / part / "done.txt").write_text(part)
            if os.environ.get("MOCK_PART_SHARED") and part in together:
                shared.write_text(part + "\n")
            text = f"Part {part} ready"
        else:
            if parallel:
                meeting = Path(os.environ["MOCK_RENDEZVOUS"])
                (meeting / participant).write_text("started")
                deadline = time.time() + 25
                while len(list(meeting.iterdir())) < int(os.environ["MOCK_RENDEZVOUS_COUNT"]):
                    assert time.time() < deadline, "implementers did not run concurrently"
                    time.sleep(0.05)
                assert "Paths you own" in prompt
                (root / f"{participant}.txt").write_text(participant)
                (root / "shared.txt").write_text(participant + "\n")
            elif os.environ.get("MOCK_COLLAB_EDIT"):
                (root / "seed.txt").write_text(os.environ["MOCK_COLLAB_EDIT"])
            if os.environ.get("MOCK_USER_EDIT"):
                # Simulates the user editing the original working tree meanwhile.
                Path(os.environ["MOCK_USER_EDIT"]).write_text(os.environ["MOCK_USER_EDIT_CONTENT"])
            (root / "completed.txt").write_text("implementation")
        if messages:
            text += "\n" + json.dumps({"messages": messages})
        send({"method": "session/update", "params": {"sessionId": session, "update": {
            "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})
        result(request, {"stopReason": "end_turn", "usage": {"totalTokens": 150, "inputTokens": 100, "outputTokens": 50}})
        return
    if behavior == "requires_handoff":
        prompt = request["params"]["prompt"][0]["text"]
        if '"current_status"' not in prompt or not (root / "partial.txt").exists():
            send({"id": request["id"], "error": {"code": -32603, "message": "missing handoff or partial changes"}})
            return
    if os.environ.get("MOCK_TOOL"):
        def update(value):
            send({"method": "session/update", "params": {"sessionId": session, "update": value}})
        pause = float(os.environ.get("MOCK_TOOL_DELAY", "0"))
        def say(text):
            update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}})
        update({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "**Inspecting the fixture**\n\nChecking files."}})
        say("I'll list the ")
        time.sleep(pause)
        say("files first.\n")
        update({"sessionUpdate": "plan", "entries": [
            {"content": "Inspect files", "priority": "high", "status": "completed"},
            {"content": "Report back", "priority": "high", "status": "in_progress"}]})
        update({"sessionUpdate": "tool_call", "toolCallId": "t1", "title": "List fixture files", "kind": "execute", "status": "in_progress",
                "rawInput": {"command": ["bash", "-lc", "ls"]}})
        time.sleep(pause)
        update({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed",
                "content": [{"type": "content", "content": {"type": "text", "text": "a.txt\nb.txt"}}]})
        update({"sessionUpdate": "tool_call", "toolCallId": "t2", "title": "Run broken tool", "kind": "execute", "status": "pending"})
        update({"sessionUpdate": "tool_call_update", "toolCallId": "t2", "status": "failed"})
        update({"sessionUpdate": "tool_call", "toolCallId": "t3", "title": "Edit notes", "kind": "edit", "status": "pending",
                "rawInput": {"file_path": "notes.txt"}})
        update({"sessionUpdate": "tool_call_update", "toolCallId": "t3", "status": "completed", "content": [
            {"type": "diff", "path": "notes.txt", "oldText": "keep\nold line\n", "newText": "keep\nnew line\n"}]})
        say("## Summary\nThe notes now use **the new line**, see `notes.txt`:\n\n```sh\ncat notes.txt\n```\n- one item\n")
    if behavior == "live_e2e":
        (root / "result.txt").write_text("OROCHI_E2E_OK\n")
    elif not os.environ.get("MOCK_READ_ONLY"):
        (root / "completed.txt").write_text("mock success\n")
    for i in range(int(os.environ.get("MOCK_LINES", "0"))):
        for part in (f"line {i + 1}: ", "the fixture keeps talking", "\n"):
            send({"method": "session/update", "params": {"sessionId": session, "update": {
                "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": part}}}})
            time.sleep(float(os.environ.get("MOCK_LINE_DELAY", "0.01")))
    send({"method": "session/update", "params": {"sessionId": session, "update": {
        "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Fixture completed."}}}})
    time.sleep(float(os.environ.get("MOCK_FINISH_DELAY", "0")))
    result(request, {"stopReason": "end_turn", "usage": {
        "totalTokens": 150, "inputTokens": 100, "outputTokens": 50, "thoughtTokens": 20, "cachedReadTokens": 30}})


for line in sys.stdin:
    request = json.loads(line)
    if log_path:
        with open(log_path, "a") as log:
            log.write(json.dumps(request) + "\n")
    method = request.get("method")
    params = request.get("params", {})
    if method == "initialize":
        if behavior == "auth":
            send({"id": request["id"], "error": {"code": -32000, "message": "authentication required"}})
        elif behavior == "broken":
            sys.exit(2)
        else:
            result(request, {"protocolVersion": 1, "agentCapabilities": {"loadSession": not os.environ.get("MOCK_NO_LOAD"),
                "promptCapabilities": {"image": not os.environ.get("MOCK_NO_IMAGE"), "embeddedContext": True}}, "agentInfo": {"name": "fixture", "version": "1"}, "authMethods": []})
    elif method in ("session/new", "session/load"):
        root = Path(params["cwd"])
        mcp_servers = params.get("mcpServers", [])
        if method == "session/load":
            session = params["sessionId"]
        response = {"sessionId": session}
        if behavior == "legacy":
            response["models"] = {"currentModelId": model, "availableModels": [{"modelId": m, "name": m} for m in models]}
        elif behavior != "default_only":
            response["configOptions"] = options()
        result(request, response)
    elif method == "session/set_config_option":
        if params["configId"] == "engine":
            if behavior == "blocked_unselected" and "astra" in params["value"]:
                send({"id":request["id"],"error":{"code":-32603,"message":"Internal error","data":{"details":"rate_limit_error", "scope":os.environ.get("MOCK_DISCOVERY_SCOPE", "agent")}}})
                continue
            model = params["value"]
            reasoning = "high" if "astra" in model else "medium"
        elif params["configId"] == "effort-control":
            allowed = ["high", "xhigh"] if "astra" in model else ["low", "medium"]
            if params["value"] not in allowed:
                send({"id": request["id"], "error": {"code": -32602, "message": "invalid effort for model"}})
                continue
            reasoning = params["value"]
        elif params["configId"] == "mode":
            mode = params["value"]
        result(request, {"configOptions": options()})
    elif method == "session/set_model":
        model = params["modelId"]
        result(request, {})
    elif method == "session/prompt":
        if os.environ.get("MOCK_DISTILLED") and "You are reviewing a coding session" in request["params"]["prompt"][0]["text"]:
            send({"method": "session/update", "params": {"sessionId": session, "update": {
                "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": os.environ["MOCK_DISTILLED"]}}}})
            result(request, {"stopReason": "end_turn", "usage": {"totalTokens": 20, "inputTokens": 15, "outputTokens": 5}})
        elif os.environ.get("MOCK_CLASSIFICATION") and "You are a task classifier" in request["params"]["prompt"][0]["text"]:
            # One fixture serves as both the classifier and the executing agent: only the
            # classification request gets the labels back.
            time.sleep(float(os.environ.get("MOCK_CLASSIFY_DELAY", "0")))
            send({"method": "session/update", "params": {"sessionId": session, "update": {
                "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": os.environ["MOCK_CLASSIFICATION"]}}}})
            result(request, {"stopReason": "end_turn", "usage": {"totalTokens": 20, "inputTokens": 15, "outputTokens": 5}})
        elif os.environ.get("MOCK_DESIGN_REPLY") and "Plan the work first" in request["params"]["prompt"][0]["text"]:
            # Streamed in small pieces, so whatever reads it sees every boundary.
            reply = os.environ["MOCK_DESIGN_REPLY"]
            for start in range(0, len(reply), 5):
                send({"method": "session/update", "params": {"sessionId": session, "update": {
                    "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": reply[start:start + 5]}}}})
            result(request, {"stopReason": "end_turn", "usage": {"totalTokens": 40, "inputTokens": 30, "outputTokens": 10}})
        elif behavior == "mailbox_chat":
            mailbox_chat(request)
            finish(request)
        elif behavior == "seats":
            try:
                seats(request)
            except BaseException:
                record_failure()
                raise
            finish(request)
        elif behavior == "adviser":
            # MOCK_VOTES is consumed per turn; MOCK_COUNTER shares the position across sessions.
            if os.environ.get("MOCK_COUNTER"):
                counter = Path(os.environ["MOCK_COUNTER"])
                turn = int(counter.read_text()) if counter.exists() else 0
                counter.write_text(str(turn + 1))
            else:
                turn = adviser_turns
            adviser_turns += 1
            votes = os.environ.get("MOCK_VOTES", "LAST").split(",")
            vote = votes[min(turn, len(votes) - 1)]
            text = request["params"]["prompt"][0]["text"]
            adviser_ids = re.findall(r'"id":"([^"]+)"', text) or adviser_ids
            if vote in ("RATE_LIMIT", "CREDIT"):
                send({"id": request["id"], "error": {"code": 429 if vote == "RATE_LIMIT" else 402,
                    "message": "rate limit exceeded" if vote == "RATE_LIMIT" else "credit balance is too low"}})
                continue
            if vote == "LAST":
                vote = adviser_ids[-1]
            send({"method": "session/update", "params": {"sessionId": session, "update": {
                "sessionUpdate": "agent_message_chunk", "content": {"type": "text",
                "text": "CLI notice\n" + json.dumps({"candidate_id": vote})}}}})
            result(request, {"stopReason": "end_turn", "usage": {"totalTokens": 900, "inputTokens": 850, "outputTokens": 50}})
        elif behavior == "retry_once" and not (root / "partial.txt").exists():
            (root / "partial.txt").write_text("unfinished work\n")
            result(request, {"stopReason": "max_tokens"})
        elif behavior == "rate" or (behavior == "model_rate" and "sol" in model):
            (root / "partial.txt").write_text("partial work\n")
            send({"id": request["id"], "error": {"code": 429, "message": "rate limit exceeded", "data": {
                "resetAt": int(time.time()) + 3600, "scope": "model" if behavior == "model_rate" else "agent"}}})
        elif behavior == "hang":
            child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
            if os.environ.get("MOCK_DETACHED"):
                # A tool process that left the agent's process group.
                detached = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"], start_new_session=True)
                (root / "detached.pid").write_text(str(detached.pid))
            (root / "agent.pid").write_text(str(os.getpid()))
            (root / "child.pid").write_text(str(child.pid))
            time.sleep(60)
        elif behavior == "permission_persist":
            # A real agent told "no" may simply try something else. Only a cancelled turn stops it.
            pending = request
            send({"id": "permission-1", "method": "session/request_permission", "params": {
                "sessionId": session, "toolCall": {"toolCallId": "call-1", "title": "Write fixture file", "kind": "edit"},
                "options": [{"optionId": "deny", "name": "Deny", "kind": "reject_once"}, {"optionId": "allow", "name": "Allow", "kind": "allow_once"}]}})
            import select as wait

            def heard():
                line = json.loads(sys.stdin.readline())
                if log_path:
                    with open(log_path, "a") as log:
                        log.write(json.dumps(line) + "\n")
                return line
            # The answer and a cancellation may arrive in either order.
            answer, cancelled = None, False
            while answer is None:
                line = heard()
                if line.get("method") == "session/cancel":
                    cancelled = True
                elif line.get("id") == "permission-1":
                    answer = line
            if answer.get("result", {}).get("outcome", {}).get("optionId") == "allow":
                finish(pending)
                continue
            if not cancelled and wait.select([sys.stdin], [], [], 1.5)[0]:
                cancelled = heard().get("method") == "session/cancel"
            if cancelled:
                result(pending, {"stopReason": "cancelled"})
                continue
            (root / "alternative.txt").write_text("tried another way\n")
            finish(pending)
        elif behavior == "permission":
            pending = request
            send({"id": "permission-1", "method": "session/request_permission", "params": {
                "sessionId": session, "toolCall": {"toolCallId": "call-1", "title": "Write fixture file", "kind": "edit"},
                "options": [{"optionId": "deny", "name": "Deny", "kind": "reject_once"}, {"optionId": "allow", "name": "Allow", "kind": "allow_once"}]}})
        elif behavior == "incomplete":
            result(request, {"stopReason": "max_tokens"})
        else:
            finish(request)
    elif method == "session/cancel":
        sys.exit(0)
    elif request.get("id") == "permission-1":
        if request.get("result", {}).get("outcome", {}).get("optionId") == "allow":
            finish(pending)
        else:
            result(pending, {"stopReason": "cancelled"})
    elif "id" in request:
        send({"id": request["id"], "error": {"code": -32601, "message": "method not found"}})
