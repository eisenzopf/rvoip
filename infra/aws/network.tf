# Dedicated release VPC: one public subnet per availability zone, an internet
# gateway, and an egress-only security group. Workers get a public IPv4
# address for outbound access only.

data "aws_availability_zones" "available" {
  state = "available"

  # Exclude Local Zones and Wavelength Zones, which need explicit opt-in.
  filter {
    name   = "opt-in-status"
    values = ["opt-in-not-required"]
  }
}

locals {
  # Stable az -> index map so cidrsubnet() offsets do not shift when AZs
  # are added; sort() keeps "first subnet" deterministic across runs.
  availability_zones = { for idx, az in sort(data.aws_availability_zones.available.names) : az => idx }
}

resource "aws_vpc" "release" {
  cidr_block           = var.vpc_cidr
  enable_dns_support   = true
  enable_dns_hostnames = true

  tags = {
    Name = "${var.name_prefix}-vpc"
  }
}

resource "aws_internet_gateway" "release" {
  vpc_id = aws_vpc.release.id

  tags = {
    Name = "${var.name_prefix}-igw"
  }
}

resource "aws_subnet" "public" {
  for_each = local.availability_zones

  vpc_id                  = aws_vpc.release.id
  availability_zone       = each.key
  cidr_block              = cidrsubnet(var.vpc_cidr, 8, each.value)
  map_public_ip_on_launch = true

  tags = {
    Name = "${var.name_prefix}-public-${each.key}"
  }
}

resource "aws_route_table" "public" {
  vpc_id = aws_vpc.release.id

  tags = {
    Name = "${var.name_prefix}-public"
  }
}

resource "aws_route" "public_default" {
  route_table_id         = aws_route_table.public.id
  destination_cidr_block = "0.0.0.0/0"
  gateway_id             = aws_internet_gateway.release.id
}

resource "aws_route_table_association" "public" {
  for_each = aws_subnet.public

  subnet_id      = each.value.id
  route_table_id = aws_route_table.public.id
}

# No ingress rules. Terraform removes the implicit allow-all egress rule from
# a managed security group, so egress is re-added explicitly below.
resource "aws_security_group" "workers" {
  name        = "${var.name_prefix}-workers"
  description = "rvoip release workers: no ingress, unrestricted egress"
  vpc_id      = aws_vpc.release.id

  tags = {
    Name = "${var.name_prefix}-workers"
  }
}

resource "aws_vpc_security_group_egress_rule" "workers_all_ipv4" {
  security_group_id = aws_security_group.workers.id
  description       = "All outbound IPv4"
  ip_protocol       = "-1"
  cidr_ipv4         = "0.0.0.0/0"
}
