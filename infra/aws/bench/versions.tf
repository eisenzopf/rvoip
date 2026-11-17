terraform {
  required_version = ">= 1.6"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
  }

  # Local state (terraform.tfstate in this directory, ignored by git), kept
  # apart from ../terraform.tfstate so a bench change can never plan a change
  # to the release-qualification resources.
}

provider "aws" {
  region = var.region

  default_tags {
    tags = var.tags
  }
}
