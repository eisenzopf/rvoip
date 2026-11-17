---
section: Breaking changes
breaking: true
---
**`rvoip-amazon-connect`'s `aws-control` feature moves to aws-config
1.13 / aws-sdk-connect 1.226 / aws-smithy-types 1.9** (still exact pins).
`AwsConnectStarter::new` takes an aws-config 1.13 `SdkConfig`.
Migration: load your `SdkConfig` with aws-config 1.13.
