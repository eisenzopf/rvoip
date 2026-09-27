terraform {
  required_version = ">= 1.6"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
    archive = {
      source  = "hashicorp/archive"
      version = "~> 2.4"
    }
  }

  # Local state by default. For shared use, uncomment and fill in an S3
  # backend (see README.md, "State"):
  #
  # backend "s3" {
  #   bucket       = "rvoip-release-terraform-state-<account_id>"
  #   key          = "infra/aws/terraform.tfstate"
  #   region       = "us-west-2"
  #   use_lockfile = true
  #   encrypt      = true
  # }
}

provider "aws" {
  region = var.region

  default_tags {
    tags = var.tags
  }
}
