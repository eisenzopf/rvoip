# One output per repository variable read by release-qualify.yml
# (docs/AWS_RELEASE_WORKERS.md, "Account resources"). README.md shows how to
# push them with `gh variable set`.

output "rvoip_aws_region" {
  description = "RVOIP_AWS_REGION: region the fleet runs in."
  value       = var.region
}

output "rvoip_aws_provisioner_role_arn" {
  description = "RVOIP_AWS_PROVISIONER_ROLE_ARN: role the controller assumes through OIDC."
  value       = aws_iam_role.provisioner.arn
}

output "rvoip_aws_runner_instance_profile" {
  description = "RVOIP_AWS_RUNNER_INSTANCE_PROFILE: instance profile name attached to workers."
  value       = aws_iam_instance_profile.runner.name
}

output "rvoip_aws_subnet_id" {
  description = "RVOIP_AWS_SUBNET_ID: release subnet (first public subnet, by AZ name)."
  value       = local.subnet_ids[0]
}

output "rvoip_aws_subnet_ids" {
  description = "All public subnet ids, one per availability zone, ordered by AZ name."
  value       = local.subnet_ids
}

output "rvoip_aws_security_group_id" {
  description = "RVOIP_AWS_SECURITY_GROUP_ID: egress-only worker security group."
  value       = aws_security_group.workers.id
}

output "rvoip_aws_evidence_bucket" {
  description = "RVOIP_AWS_EVIDENCE_BUCKET: immutable results, logs and bundles."
  value       = aws_s3_bucket.evidence.bucket
}

output "rvoip_aws_cache_bucket" {
  description = "RVOIP_AWS_CACHE_BUCKET: sccache backend."
  value       = aws_s3_bucket.cache.bucket
}

output "rvoip_aws_ami_ssm_parameter" {
  description = "RVOIP_AWS_AMI_SSM_PARAMETER: SSM public parameter the workflow resolves to the Ubuntu AMI."
  value       = local.ami_ssm_parameter
}

locals {
  subnet_ids = [for az in sort(keys(aws_subnet.public)) : aws_subnet.public[az].id]
}
