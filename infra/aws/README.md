# rvoip release workers: AWS account resources

Terraform for everything the `Release qualification` workflow needs to run
its ephemeral EC2 fleet in the Vapi account (`us-west-2`). The design and
the resource inventory are in [`docs/AWS_RELEASE_WORKERS.md`](../../docs/AWS_RELEASE_WORKERS.md);
this README only covers operating the module.

What it creates (all tagged `rvoip-release=true`):

| File | Resources |
| --- | --- |
| `iam.tf` | `rvoip-release-gh-provisioner` (OIDC-assumed by the workflow), `rvoip-release-runner` role + instance profile |
| `s3.tf` | evidence bucket (versioned, `release-cache/` expires after 14 d), compiler cache bucket (expires after 30 d) |
| `network.tf` | dedicated VPC, one public subnet per AZ, internet gateway, egress-only security group |
| `janitor.tf` + `lambda/janitor.py` | Lambda on an EventBridge schedule that terminates workers past their `rvoip-expires-at` tag |

It references, and never creates, the account's existing
`token.actions.githubusercontent.com` OIDC provider.

## Apply

Uses the an SSO profile with administrator access SSO profile. Terraform >= 1.6 and the AWS provider 6.x.

```sh
aws sso login --profile <admin-profile>
cd infra/aws
AWS_PROFILE=<admin-profile> terraform init
AWS_PROFILE=<admin-profile> terraform plan -out plan.tfplan
# review the plan
AWS_PROFILE=<admin-profile> terraform apply plan.tfplan
```

Re-running `plan` after a change to `lambda/janitor.py` picks the new code up
automatically (the zip is rebuilt into `.build/` and its hash is tracked).

## One-time GitHub OIDC `sub` customisation

The provisioner trust policy matches the token's `sub` claim against

```
repo:eisenzopf/rvoip:job_workflow_ref:eisenzopf/rvoip/.github/workflows/release-qualify.yml@refs/heads/*
```

GitHub's default `sub` is `repo:<owner>/<repo>:ref:<ref>` (or
`:environment:<env>` / `:pull_request`), which does not carry the workflow
path, so the repository has to opt into a custom claim shape once:

```sh
gh api --method PUT repos/eisenzopf/rvoip/actions/oidc/customization/sub \
  -f use_default=false \
  -f 'include_claim_keys[]=repo' \
  -f 'include_claim_keys[]=job_workflow_ref'
```

This is repository-wide: **every** workflow in `eisenzopf/rvoip` now mints
tokens with `sub = repo:<owner/repo>:job_workflow_ref:<owner/repo>/<path>@<ref>`.
Any other trust policy that matched the default `sub` (in this or any other
cloud account) must be updated at the same time. The upside is that the role
can only be assumed from the named workflow file on a branch ref, not from an
arbitrary workflow that happens to run in the repository.

To verify the claim shape before relying on it, add a temporary step to any
workflow with `permissions: id-token: write` and read the token's payload:

```yaml
- name: Print OIDC claims
  run: |
    TOKEN=$(curl -sS -H "Authorization: bearer $ACTIONS_ID_TOKEN_REQUEST_TOKEN" \
      "$ACTIONS_ID_TOKEN_REQUEST_URL&audience=sts.amazonaws.com" | jq -r .value)
    echo "$TOKEN" | cut -d. -f2 | base64 -d 2>/dev/null | jq '{sub, aud, repository, job_workflow_ref}'
```

The printed `sub` must be exactly the string above with the branch filled in.
Check the current setting with
`gh api repos/eisenzopf/rvoip/actions/oidc/customization/sub`.

## Push the outputs to repository variables

Each output maps one-to-one onto a repository variable the workflow reads:

```sh
cd infra/aws
AWS_PROFILE=<admin-profile> terraform output -json \
  | jq -r 'to_entries[]
           | select(.key | startswith("rvoip_aws_"))
           | select(.key != "rvoip_aws_subnet_ids")
           | "\(.key | ascii_upcase) \(.value.value)"' \
  | while read -r name value; do
      gh variable set "$name" --repo eisenzopf/rvoip --body "$value"
    done
```

`rvoip_aws_subnet_ids` (the full per-AZ list) is informational and skipped;
the workflow reads the single `RVOIP_AWS_SUBNET_ID`. List what landed with
`gh variable list --repo eisenzopf/rvoip`.

## What the controller must do to satisfy the IAM policy

- `run-instances` must include `TagSpecifications` for **both** `instance`
  and `volume` resource types, each with `managed-by=github-actions`. The
  policy conditions `ec2:RunInstances` on that request tag for instances and
  volumes, so an untagged root volume is denied.
- `rvoip-expires-at` must be ISO 8601 in UTC (`2026-09-27T14:00:00Z` or
  `+00:00`). The janitor skips, and logs, instances whose tag is missing or
  unparseable; it never guesses.
- Pass the instance profile by name (`RVOIP_AWS_RUNNER_INSTANCE_PROFILE`); the
  provisioner may `iam:PassRole` only the runner role, only to EC2.

## State

State is local by default (`terraform.tfstate` in this directory, ignored by
git). That is fine for a single operator; for shared or CI-driven applies,
create a state bucket once by hand and uncomment the `backend "s3"` block in
`versions.tf` (it uses the S3 native lockfile, so no DynamoDB table is
needed), then run `terraform init -migrate-state`.

The provider lock file `.terraform.lock.hcl` is committed so every apply uses
the same provider build.

## Destroy

Nothing here is stateful except the evidence bucket, which holds signed
release receipts. Empty and delete it deliberately, do not let `destroy` do it:

```sh
AWS_PROFILE=<admin-profile> terraform state rm aws_s3_bucket.evidence   # keep the evidence
AWS_PROFILE=<admin-profile> terraform destroy
```

The cache bucket is destroyed with the rest (it is rebuilt by the next run).
If `destroy` stalls on a bucket, the bucket is not empty: the module does not
set `force_destroy`, on purpose.
