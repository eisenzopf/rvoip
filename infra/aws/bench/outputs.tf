# run_lane.py reads these with `terraform output -json` (or from
# scripts/bench/lane.json, which records them per environment).

output "region" {
  value = var.region
}

output "availability_zone" {
  value = var.availability_zone
}

output "dut_launch_template" {
  value = aws_launch_template.dut.name
}

output "caller_launch_template" {
  value = aws_launch_template.caller.name
}

output "host_instance_profile" {
  value = aws_iam_instance_profile.host.name
}

output "orchestrator_policy_arn" {
  value = aws_iam_policy.orchestrator.arn
}

output "security_group_id" {
  value = aws_security_group.bench.id
}

output "subnet_id" {
  value = data.aws_subnet.lane.id
}

output "placement_group" {
  value = aws_placement_group.lane.name
}

output "evidence_bucket" {
  value = local.evidence_bucket
}

output "ami_id" {
  value = local.ami_id
}

output "lane_vcpus" {
  value = local.lane_vcpus
}

output "caller_sources" {
  value = var.caller_sources
}

output "dut_instance_type" {
  value = var.dut_instance_type
}

output "caller_instance_type" {
  value = var.caller_instance_type
}
