variable "region" {
  description = "AWS region of the bench lane (the release VPC's region)."
  type        = string
  default     = "us-west-2"
}

variable "availability_zone" {
  description = "AZ of the lane. Must offer the DUT and caller instance types (checked below); the cluster placement group lives here."
  type        = string
  default     = "us-west-2b"
}

variable "name_prefix" {
  description = "Prefix of every named bench resource."
  type        = string
  default     = "rvoip-bench"
}

variable "release_name_prefix" {
  description = "name_prefix of the ../ (release) root, used to find its VPC, subnets and evidence bucket."
  type        = string
  default     = "rvoip-release"
}

variable "bench_ami_id" {
  description = "Baked bench AMI (scripts/bench/ami/bake.py). Null uses stock Ubuntu 24.04, which is only fit for baking. A new AMI is a new bench environment (env-id): re-pin and re-baseline."
  type        = string
  # rvoip-bench-20261117T010818Z-1b56903e410c (Ubuntu 24.04, scripts/bench/ami digest 1b56903e410c)
  default = "ami-0d3897eb748e4b60e"
}

variable "dut_instance_type" {
  description = "Device under test. c8i.4xlarge matches the E74 final report."
  type        = string
  default     = "c8i.4xlarge"
}

variable "caller_instance_type" {
  description = "Load generator (and ea-uas host for the testbed baseline)."
  type        = string
  default     = "c8in.24xlarge"
}

variable "caller_count" {
  description = "Caller hosts per lane (the vCPU check uses it)."
  type        = number
  default     = 2
}

variable "caller_sources" {
  description = "Source IPs per caller host: the primary private IP plus caller_sources - 1 secondaries."
  type        = number
  default     = 2

  validation {
    condition     = var.caller_sources >= 1 && var.caller_sources <= 8
    error_message = "caller_sources must be 1-8 (the caller drives at most 8 source IPs)."
  }
}

variable "dut_root_gb" {
  type    = number
  default = 100
}

variable "caller_root_gb" {
  description = "Caller root volume: evidence of a 180 s trial is about 8 GB before upload."
  type        = number
  default     = 300
}

variable "qualification_vcpus" {
  description = "On-demand vCPUs the release qualification fleet may hold at the same time (left free by the quota check)."
  type        = number
  default     = 192
}

variable "tags" {
  description = "Tags on every bench resource (provider default_tags)."
  type        = map(string)
  default = {
    rvoip-bench = "true"
  }
}

variable "release_provisioner_role_name" {
  description = "IAM role the Release qualification workflow assumes over OIDC (vars.RVOIP_AWS_PROVISIONER_ROLE_ARN). Its bench legs, gate-bench and the bench cleanup job run run_lane.py as this role, so the orchestrator policy is attached to it. Empty: attach nothing."
  type        = string
  default     = "rvoip-release-gh-provisioner"
}
