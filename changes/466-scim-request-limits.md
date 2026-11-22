---
section: Security
breaking: true
---
SCIM now enforces a request body limit inside the SCIM router (default 2 MiB,
set with `ScimConfig::with_max_request_body_bytes`). The router wraps each
request body in the limit, so it stacks with any limit the application sets
where it mounts the router and the smallest wins. An outer
`DefaultBodyLimit::disable()` or a larger outer limit no longer widens the
limit on SCIM routes, and a smaller outer `DefaultBodyLimit` keeps working.
axum's own 2 MiB extractor limit still applies too, so to accept SCIM bodies
over 2 MiB, raise the SCIM limit and also raise or disable `DefaultBodyLimit`
where the router is mounted. A larger body gets `413 Payload Too Large`.

SCIM also caps the group members one request may carry (default 10,000, set
with `ScimConfig::with_max_group_members_per_request`) when it creates or
replaces a Group, or when a PATCH replaces `members`. Over the cap, the router
returns a `400` SCIM error with `scimType` `invalidValue`, and the service
methods return the new `ScimError::InvalidValue`.

Breaking changes and migration:

- If you mounted the SCIM router with `DefaultBodyLimit::disable()` (or a
  limit above 2 MiB) and accepted SCIM bodies over 2 MiB, they now get `413`.
  Raise `ScimConfig::with_max_request_body_bytes` to the size you accept; your
  mount already lifts axum's 2 MiB extractor limit.
- If you provision groups with more than 10,000 members in one request, raise
  `ScimConfig::with_max_group_members_per_request`. The adapter's PATCH
  supports only `replace` of `members`, which carries the whole membership, so
  splitting a group across PATCH requests does not get past the cap.
- An exhaustive `match` on `ScimError` needs an arm for `InvalidValue`.
