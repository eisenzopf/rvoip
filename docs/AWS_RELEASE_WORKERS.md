# AWS release workers

The release qualification fleet runs on ephemeral EC2 instances in the Vapi
AWS account (region `us-west-2`) instead of Google Compute Engine. The
account id, role ARNs, and bucket names are Terraform outputs and repository
variables, not repository content. The orchestration model is unchanged: one GitHub-hosted controller
plans duration-balanced shards, creates every worker concurrently, waits for
each worker's immutable result in S3, verifies and merges the evidence, and
terminates every instance. There is no idle fleet and no long-lived cloud
credential in GitHub.

## Machine classes

N2 workers were Intel Cascade Lake with 4 GB per vCPU. The `m5` family is the
same generation (Skylake-SP or Cascade Lake, 3.1 GHz, 4 GB per vCPU). AWS does
not expose a minimum CPU platform; every worker records the actual CPU model in
its evidence as before.

| Resource class | Instance type | vCPU | Memory | Root volume | Was |
| --- | --- | ---: | ---: | --- | --- |
| `ec2-performance` | `m5.2xlarge` | 8 | 32 GB | 200 GB gp3 | `gcp-performance` / `n2-standard-8` |
| `ec2-performance-soak` | `m5.xlarge` | 4 | 16 GB | 200 GB gp3 | `gcp-performance-soak` / `n2-standard-4` |
| `ec2-performance-soak-long` | `m5.2xlarge` | 8 | 32 GB | 200 GB gp3 | `gcp-performance-soak-long` / `n2-standard-8` |
| `ec2-interop` | `m5.xlarge` | 4 | 16 GB | 200 GB gp3 | `gcp-interop` / `n2-standard-4` |
| `ec2-proxy-interop` | `m5.large` | 2 | 8 GB | 100 GB gp3 | `gcp-proxy-interop` / `n2-standard-2` |
| performance prebuilder | `m5.8xlarge` | 32 | 128 GB | 200 GB gp3 | `n2-standard-32` |

The full `remote-release` shape is unchanged: six short-performance workers,
two long-soak workers, seven burst/soak workers, one interoperability worker,
and two proxy-interoperability workers, 100 vCPUs concurrently, plus the
32-vCPU builder that runs and is terminated before the fleet starts. Peak
demand is therefore 100 On-Demand Standard vCPUs against the account's
1,152-vCPU quota in `us-west-2`.

Machine classes are policy values recorded in every attestation. Moving clouds
changes the environment, so the release environment identifier becomes
`rvoip-release-v6-rust-1.91-nextest-0.9.140-prebuilt-perf-v2-lld-ec2-m5` and
every environment-sensitive gate runs fresh on the first AWS qualification.
Performance thresholds are unchanged; the first AWS run establishes whether
`m5` meets them and becomes the comparison baseline for later releases.

## Account resources (Terraform, `infra/aws`)

Everything the fleet needs is created by the Terraform module in
[`infra/aws`](../infra/aws) and tagged `rvoip-release=true`:

- **GitHub OIDC trust.** The account already has the
  `token.actions.githubusercontent.com` provider; the module references it.
  The `rvoip-gh-provisioner` role trusts only the `Release qualification`
  workflow of `eisenzopf/rvoip`, matched on the customised `sub` claim
  `repo:eisenzopf/rvoip:job_workflow_ref:eisenzopf/rvoip/.github/workflows/release-qualify.yml@refs/heads/*`.
  The repository must enable that claim shape once (see the module README).
- **Provisioner role.** May run, describe, tag, stop, and terminate instances
  that carry `managed-by=github-actions`, pass the runner instance role, read
  the Ubuntu AMI SSM parameter, read service quotas, and read the evidence and
  cache buckets. It cannot write evidence.
- **Runner instance role and profile** (`rvoip-release-runner`). May put and
  get objects under the evidence bucket and read/write the compiler cache
  bucket. No EC2 permissions.
- **Evidence bucket.** Versioned, private, SSE-S3, lifecycle expires only the
  `release-cache/` prefix after 14 days. Run-scoped receipts and logs are
  durable.
- **Compiler cache bucket.** Private, lifecycle expires objects after 30 days.
- **Release VPC.** A dedicated VPC with one public subnet per availability
  zone, an internet gateway, and a security group with no ingress rules and
  unrestricted egress. Workers get a public IPv4 address for outbound access
  only; IMDSv2 is required and instance tags are readable from IMDS.
- **Janitor.** An EventBridge schedule invokes a small Lambda every 30 minutes
  that terminates any `managed-by=github-actions` instance whose
  `rvoip-expires-at` tag is in the past. Controllers set the tag to four hours
  after creation.

The module outputs map directly onto the repository variables the workflow
reads:

| Variable | Meaning |
| --- | --- |
| `RVOIP_AWS_REGION` | `us-west-2` |
| `RVOIP_AWS_PROVISIONER_ROLE_ARN` | Role the controller assumes through OIDC |
| `RVOIP_AWS_RUNNER_INSTANCE_PROFILE` | Instance profile name attached to workers |
| `RVOIP_AWS_SUBNET_ID` | Release subnet |
| `RVOIP_AWS_SECURITY_GROUP_ID` | Egress-only security group |
| `RVOIP_AWS_EVIDENCE_BUCKET` | Immutable results, logs, bundles |
| `RVOIP_AWS_CACHE_BUCKET` | sccache backend |
| `RVOIP_AWS_AMI_SSM_PARAMETER` | `/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id` |

## Worker contract

The controller checks out the exact candidate and builds each worker's
user-data with `scripts/release/aws_fanout.py user-data`. User-data is a
gzip-compressed shell script (EC2 accepts up to 16 KiB) that:

1. writes `/etc/rvoip-release.env` (mode 0600) with the worker parameters;
2. installs the reviewed startup and shutdown scripts from the controller's
   checkout under `/usr/local/lib/rvoip-release/`;
3. installs and starts `rvoip-release-shutdown.service`, a oneshot unit whose
   `ExecStop` runs the shutdown checkpoint, so a controller `stop-instances`
   (an ACPI shutdown) snapshots partial evidence exactly as the GCE shutdown
   script did;
4. runs the startup script.

`/etc/rvoip-release.env` keys (all present on every worker; empty when unused):

```
RVOIP_AWS_REGION
RVOIP_CANDIDATE
RVOIP_RUN_ID                    # <github_run_id>-<run_attempt>
RVOIP_SHARD_ID
RVOIP_RESOURCE_CLASS
RVOIP_EVIDENCE_BUCKET
RVOIP_CACHE_BUCKET
RVOIP_PREFIX                    # release/<run>/<shard>
RVOIP_GATES_B64
RVOIP_ENVIRONMENT_B64
RVOIP_PREBUILT_URI              # s3://bucket/key or empty
RVOIP_PREBUILT_SHA256
RVOIP_EXTERNAL_MEMORY_DIAGNOSTICS   # 0 or 1
RVOIP_MIMALLOC_ALLOW_THP        # 0, 1, or empty
RVOIP_PREBUILD_CACHE_KEY        # prebuilder only
```

Object layout in the evidence bucket is unchanged from GCS, with `s3://` in
place of `gs://`:

```
s3://EVIDENCE/release/<run>-<attempt>/<shard>/result.json
s3://EVIDENCE/release/<run>-<attempt>/<shard>/release-shard.tar.gz
s3://EVIDENCE/release/<run>-<attempt>/<shard>/qualification.log
s3://EVIDENCE/release/<run>-<attempt>/prebuild/{prebuild-result.json,prebuild.log}
s3://EVIDENCE/release-cache/performance-prebuilt-v1/<cache-key>/...
```

Workers upload and download with the AWS CLI using the instance role. The
CLI is installed from the pinned official archive and its signature is
verified with the AWS CLI public key before use. `sccache` uses the S3
backend (`SCCACHE_BUCKET`, `SCCACHE_REGION`, `SCCACHE_S3_KEY_PREFIX`) with the
same key prefix and the same fall-back-to-direct-rustc behaviour.

Instances are created with `InstanceInitiatedShutdownBehavior=terminate` and a
delete-on-termination root volume, so a worker's own `shutdown -h now` after
uploading its result terminates it and frees the disk. The controller treats
`stopping`, `stopped`, `shutting-down`, and `terminated` as "worker finished"
when it polls, mirroring GCE `TERMINATED`.

Every instance carries these tags:

```
Name=rvoip-rel-<run>-<attempt>-<shard>
managed-by=github-actions
rvoip-role=release-gate | release-prebuild
rvoip-run-id=<run>-<attempt>
rvoip-candidate=<sha>
rvoip-shard-id=<shard>
rvoip-resource-class=<class>
rvoip-expires-at=<ISO 8601 UTC, creation + 4h>
```

## Workflow shape

`release-qualify.yml` keeps the same jobs with the cloud swapped:

- `preflight-aws` assumes the provisioner role, reads the On-Demand Standard
  vCPU quota and current usage, the gp3 storage quota, resolves the AMI, and
  fails closed if peak demand does not fit.
- `gate-aws` prepares the manifest, builds the performance bundle once on an
  `m5.8xlarge` (or reuses the content-addressed cache), creates every worker
  concurrently with `run-instances`, polls S3 and instance states, applies the
  same early-failure cutoff by stopping deferred workers, downloads and
  verifies every bundle, and terminates the fleet.
- `cleanup-aws` sweeps instances tagged with this run that an interrupted
  controller left behind and fails if any remain.

Cleanup runs in three layers as before: worker self-termination after
evidence upload, controller terminate on `always()`, and the scheduled
janitor.

## What was removed

The GCP pilot workflow, its startup script, `gcp_fanout.py`, the three GCP
runner scripts, and the `RVOIP_GCP_*` repository variables have no AWS
counterpart beyond what is listed above. The GCP project can be decommissioned
independently; nothing in the repository references it after this change.
