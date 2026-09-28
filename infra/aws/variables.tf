variable "region" {
  description = "AWS region the release fleet runs in."
  type        = string
  default     = "us-west-2"
}

variable "github_repository" {
  description = "GitHub repository (owner/name) whose release workflow may assume the provisioner role."
  type        = string
  default     = "eisenzopf/rvoip"
}

variable "release_workflow_path" {
  description = "Path, relative to the repository root, of the workflow file allowed to assume the provisioner role."
  type        = string
  default     = ".github/workflows/release-qualify.yml"
}

variable "trusted_refs_glob" {
  description = "Git ref glob (StringLike) the release workflow must run from to assume the provisioner role."
  type        = string
  default     = "refs/heads/*"
}

variable "name_prefix" {
  description = "Prefix applied to every named resource (roles, buckets, VPC, Lambda)."
  type        = string
  default     = "rvoip-release"
}

variable "vpc_cidr" {
  description = "IPv4 CIDR block of the release VPC. One /24 public subnet is carved out per availability zone."
  type        = string
  default     = "10.90.0.0/16"
}

variable "worker_expiry_check_minutes" {
  description = "How often, in minutes, the janitor Lambda sweeps for instances whose rvoip-expires-at tag is in the past."
  type        = number
  default     = 30

  validation {
    condition     = var.worker_expiry_check_minutes >= 1 && floor(var.worker_expiry_check_minutes) == var.worker_expiry_check_minutes
    error_message = "worker_expiry_check_minutes must be a whole number of minutes, at least 1."
  }
}

variable "tags" {
  description = "Tags applied to every resource the module creates (via provider default_tags)."
  type        = map(string)
  default = {
    rvoip-release = "true"
  }
}
