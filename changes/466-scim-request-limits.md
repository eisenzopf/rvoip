---
section: Security
breaking: true
---
SCIM now enforces a request body limit inside the SCIM router (default 2 MiB,
set with `ScimConfig::with_max_request_body_bytes`). It is the router's
innermost layer, so a `DefaultBodyLimit` where the application mounts the
router, disabled, raised or lowered, no longer changes the limit on SCIM routes.
A larger body gets `413 Payload Too Large`. SCIM also caps the group members
one request may carry (default 10,000, set with
`ScimConfig::with_max_group_members_per_request`) when it creates or replaces
a Group, or a PATCH replaces `members`. Over the cap, the router returns a
`400` SCIM error with `scimType` `invalidValue`, and the service methods
return the new `ScimError::InvalidValue`.

Migration: if you provision groups with more than 10,000 members in one
request, raise `ScimConfig::with_max_group_members_per_request`. The adapter's
PATCH supports only `replace` of `members`, which carries the whole membership,
so splitting a group across PATCH requests does not get past the cap. If you
relied on an outer `DefaultBodyLimit` for SCIM routes, set
`ScimConfig::with_max_request_body_bytes` instead. An exhaustive `match` on
`ScimError` needs an arm for `InvalidValue`.
