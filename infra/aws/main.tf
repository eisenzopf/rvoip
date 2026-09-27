# Shared data sources and locals. Resources are split across iam.tf, s3.tf,
# network.tf and janitor.tf; outputs are in outputs.tf.

data "aws_caller_identity" "current" {}

data "aws_partition" "current" {}

locals {
  account_id = data.aws_caller_identity.current.account_id
  partition  = data.aws_partition.current.partition

  # Every worker instance (and its root volume) carries this tag; the
  # provisioner may only create, tag, stop and terminate tagged resources and
  # the janitor only ever terminates tagged instances.
  managed_by_tag_key   = "managed-by"
  managed_by_tag_value = "github-actions"
  expires_at_tag_key   = "rvoip-expires-at"

  evidence_bucket_name = "${var.name_prefix}-evidence-${local.account_id}"
  cache_bucket_name    = "${var.name_prefix}-compiler-cache-${local.account_id}"

  # Canonical Ubuntu 24.04 LTS amd64 gp3 AMI, resolved by the workflow at run time.
  ami_ssm_parameter = "/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id"
}
