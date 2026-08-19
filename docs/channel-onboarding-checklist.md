# New channel onboarding checklist (ZER-569)

A new channel should be **adapter + presentation policy** only. Control-plane
session/profile/transcript code must not grow channel-named branches.

## Add

- [ ] `ChatAdapter` implementation (or reuse `UnifiedGatewayAdapter` / gateway protocol)
- [ ] `platform()` string; session keys become `{platform}:{thread_id}`
- [ ] `message_limit()` from the platform documented cap
- [ ] threading model mapping into `ChannelRef`
- [ ] capability probes: streaming/edit, status API, native tables, intermediate-text privacy
- [ ] `delivery_mode` via adapter transport table / `delivery_mode_for_gateway_platform` when applicable
- [ ] default presentation behaviour via adapter capability probes + optional `[presentation.<platform>]`
- [ ] `[<channel>]` config section with secure-by-default allowlists
- [ ] `docs/config-reference.md` entries and a `docs/<channel>.md` page
- [ ] adapter tests + router/presentation tests through `MockAdapter`

## Must NOT be needed

- [ ] no change to `SessionPool`, session keying, or TTL semantics
- [ ] no new Profile schema field (`agent_profile` / `profile_store`)
- [ ] no new config backend or duplicated config store
- [ ] no channel-specific branch inside `AdapterRouter::stream_prompt_blocks` (use policy values)
- [ ] no channel-specific transcript / SSE shape

If an item in the second list is unavoidable, write a follow-up ADR instead of
special-casing the router.
