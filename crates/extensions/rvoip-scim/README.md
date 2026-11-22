# rvoip-scim

SCIM 2.0 provisioning adapter for RVoIP users-core.

It maps SCIM Users and Groups onto `rvoip-users-core` users, roles, active
state, and external identity links, and serves them as an Axum router from
`ScimService::router()`. Admin requests are authorised by an
`rvoip-auth-core` `BearerValidator`, so tokens can come from users-core,
Keycloak/OIDC, OAuth2 introspection, or a custom enterprise service.

## Configuration

`ScimConfig::new(provider_id)` sets the defaults below; each has a builder
setter.

| Field | Default | Setter |
| --- | --- | --- |
| `read_scope` | `scim.read` | `with_read_scope` |
| `write_scope` | `scim.write` | `with_write_scope` |
| `default_roles` | `["user"]` | `with_default_roles` |
| `max_request_body_bytes` | 2 MiB | `with_max_request_body_bytes` |
| `max_group_members_per_request` | 10,000 | `with_max_group_members_per_request` |

`max_request_body_bytes` caps every request body the SCIM router reads. The
router applies it as its innermost layer, so it holds for SCIM routes whatever
`DefaultBodyLimit` the application sets where it mounts the router: disabling
or raising the outer limit does not widen it, and lowering the outer limit does
not narrow it. A larger body gets `413 Payload Too Large`.

`max_group_members_per_request` caps the `members` one request may carry:
POST and PUT of a Group, and a PATCH that replaces `members`. It applies to the
router and to the direct `ScimService` methods. Over the cap, the service
returns `ScimError::InvalidValue`, which the router sends as a `400` SCIM error
(RFC 7644 §3.12):

```json
{
  "schemas": ["urn:ietf:params:scim:api:messages:2.0:Error"],
  "scimType": "invalidValue",
  "detail": "a request may carry at most 10000 group members; this one carries 10001",
  "status": "400"
}
```

PATCH supports only `replace` of `members`, which carries the whole
membership, so splitting a membership across PATCH requests does not get past
the cap: to provision a larger group, raise the cap.
