# Codeg facade

This crate contains the phase-one compatibility surface that lets
`openab-gateway` host the Codeg web export without changing Codeg components.

It provides:

- static file lookup in the order `rel`, `rel.html`, `rel/index.html`,
  then `index.html`;
- the 23 observed cold-start `POST /api/{command}` compatibility
  endpoints, plus `save_opened_tabs` so a later tab persist does not 404;
- object-shaped DTOs for `app_update_status`, `app_update_state`,
  `check_app_update`, and `get_feedback_settings`;
- Bearer authentication using the existing gateway admin token; and
- the `/ws/events` Codeg subprotocol handshake and immediate
  `{"channel":"__ready__"}` event.

The facade is disabled by default. Build and run the standalone gateway with:

```bash
cargo run -p openab-gateway --no-default-features --features codeg
```

The following environment variables are required:

- `CODEG_WEB_ROOT`: absolute or relative path to Codeg's Next.js static
  `out/` directory.
- `GATEWAY_ADMIN_TOKEN` or `OPENAB_ADMIN_TOKEN`: the existing admin token.
- `CODEG_CHAT_ROOT` (optional): directory under which the phase-one
  `create_chat_dir` endpoint creates scratch directories.

Build Codeg with Node.js 22 and pnpm 11.9.0. The CI workflow checks out the
pinned `ZeroPointSix/codeg` revision, runs its postinstall hook, builds
`out/`, and packages that directory next to the gateway binary and upstream
Apache-2.0 license.
