# Cursor fixture provenance

The `.connect.hex` files are source-clear protobuf/Connect transcriptions
from OpenCodex v2.7.35 commit
`97e7326f89bcfbb29a2c73250cb25eb801d066b6`:

https://github.com/lidge-jun/opencodex/blob/97e7326f89bcfbb29a2c73250cb25eb801d066b6/src/adapters/cursor/gen/agent_pb.ts
https://github.com/lidge-jun/opencodex/blob/97e7326f89bcfbb29a2c73250cb25eb801d066b6/tests/cursor-framing.test.ts

The opt-in/default-disabled and native-local-execution-disabled behavior is normalized from:

https://github.com/lidge-jun/opencodex/blob/97e7326f89bcfbb29a2c73250cb25eb801d066b6/src/adapters/cursor/transport.ts
https://github.com/lidge-jun/opencodex/blob/97e7326f89bcfbb29a2c73250cb25eb801d066b6/src/adapters/cursor/exec-policy.ts

The generated schema supplies the exact field numbers, and the framing test
supplies the five-byte Connect envelope. Normalization: request ids, model text,
tool arguments, and usage counts are fixed synthetic values. No token, account
identifier, internal endpoint, command output, captured traffic, or production
body is present.

Independent verification was run with Python `google.protobuf` 6.33.5, not the
Rust production codec. The temporary probe defined the pinned message subset in
a dynamic `FileDescriptorProto`, parsed every response payload, and asserted
`SerializeToString(deterministic=True) == payload`; it independently encoded the
normalized request fixture. Exact invocation:

```text
python tests/fixtures/protocols/cursor/verify_fixture.py
python tests/fixtures/protocols/cursor/verify_again.py
```

The probe output identified, in order, `text_delta("Hello")`,
`thinking_delta("Checking.")`, MCP `weather/call_cursor`,
two cumulative `partial_tool_call` values ending in `{"city":"Paris"}`,
`token_delta(3)`, `turn_ended`, Connect end-stream `{}`, and the separate
`exec_server_message`. The request output was a 97-byte protobuf
payload inside a flags-zero Connect frame. The temporary probe was removed
after verification so these static fixtures do not add a generation dependency.
The second probe revalidated every protobuf payload byte-for-byte after the
two cumulative partial frames and success trailer were added; it reported
`partials=['{"city":', '{"city":"Paris"}']`, `connect_end_stream=True`, and
`bytes=239`.
