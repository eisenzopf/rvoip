# Bench lane resources for the RFC 7501/7502 release benchmark
# (crates/sip/rvoip-sip/docs/RFC7502_BENCHMARK.md). Instances are not managed
# here: scripts/bench/run_lane.py launches them from these launch templates for
# one lane run and terminates them at the end.

data "aws_caller_identity" "current" {}

data "aws_partition" "current" {}

locals {
  account_id = data.aws_caller_identity.current.account_id
  partition  = data.aws_partition.current.partition

  evidence_bucket = "${var.release_name_prefix}-evidence-${local.account_id}"
  evidence_prefix = "bench/"

  # Same tags the release janitor sweeps: a lane the orchestrator loses is
  # terminated once its rvoip-expires-at passes.
  managed_by_tag_key   = "managed-by"
  managed_by_tag_value = "github-actions"

  ubuntu_ami_parameter = "/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id"
  ami_id               = coalesce(var.bench_ami_id, data.aws_ssm_parameter.ubuntu.insecure_value)

  # SIP: UDP and TLS listeners of the DUT, ea-uas and every caller endpoint
  # (5061 + 2k, 5070 + 2k for k < 32). RTP: DUT/ea-uas 10000-29999 and the
  # callers' 30000-64999.
  sip_ports     = { from = 5060, to = 5135 }
  rtp_ports     = { from = 10000, to = 64999 }
  metrics_ports = { from = 9464, to = 9465 }
}

data "aws_ssm_parameter" "ubuntu" {
  name = local.ubuntu_ami_parameter
}

data "aws_s3_bucket" "evidence" {
  bucket = local.evidence_bucket
}
