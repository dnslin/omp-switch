# Logging policy

OMP Switch logs lifecycle events, intent-command names, completion status, safe error codes, and timing metadata.

It must not log configuration file bodies, HTTP request or response bodies, authentication values or headers, Direct API Keys, arbitrary local file contents, or complete IPC payloads. User-facing errors use the shared `AppError` shape: a concise message, stable code, and next action. Detailed diagnostics remain redacted before logging or returning through IPC.
