# The lane reuses the release VPC and its public subnet in var.availability_zone
# (hosts need outbound access for SSM and S3). Bench traffic stays inside the
# bench security group.

data "aws_vpc" "release" {
  tags = {
    Name = "${var.release_name_prefix}-vpc"
  }
}

data "aws_subnet" "lane" {
  vpc_id            = data.aws_vpc.release.id
  availability_zone = var.availability_zone

  tags = {
    Name = "${var.release_name_prefix}-public-${var.availability_zone}"
  }
}

# Every public subnet of the release VPC: run_lane.py --az launches a lane in any of them
# (the templates name only data.aws_subnet.lane).
data "aws_subnets" "release_public" {
  filter {
    name   = "vpc-id"
    values = [data.aws_vpc.release.id]
  }

  tags = {
    Name = "${var.release_name_prefix}-public-*"
  }
}

data "aws_ec2_instance_type_offerings" "lane" {
  location_type = "availability-zone"

  filter {
    name   = "location"
    values = [var.availability_zone]
  }

  filter {
    name   = "instance-type"
    values = [var.dut_instance_type, var.caller_instance_type]
  }
}

resource "aws_security_group" "bench" {
  name        = "${var.name_prefix}-lane"
  description = "rvoip bench lane: SIP, RTP and metrics between bench hosts only; no other ingress"
  vpc_id      = data.aws_vpc.release.id

  tags = {
    Name = "${var.name_prefix}-lane"
  }
}

resource "aws_vpc_security_group_ingress_rule" "sip_udp" {
  security_group_id            = aws_security_group.bench.id
  description                  = "SIP over UDP between bench hosts"
  ip_protocol                  = "udp"
  from_port                    = local.sip_ports.from
  to_port                      = local.sip_ports.to
  referenced_security_group_id = aws_security_group.bench.id
}

resource "aws_vpc_security_group_ingress_rule" "sip_tcp" {
  security_group_id            = aws_security_group.bench.id
  description                  = "SIP over TLS between bench hosts"
  ip_protocol                  = "tcp"
  from_port                    = local.sip_ports.from
  to_port                      = local.sip_ports.to
  referenced_security_group_id = aws_security_group.bench.id
}

resource "aws_vpc_security_group_ingress_rule" "rtp" {
  security_group_id            = aws_security_group.bench.id
  description                  = "RTP/SRTP (rtcp-mux) between bench hosts"
  ip_protocol                  = "udp"
  from_port                    = local.rtp_ports.from
  to_port                      = local.rtp_ports.to
  referenced_security_group_id = aws_security_group.bench.id
}

resource "aws_vpc_security_group_ingress_rule" "metrics" {
  security_group_id            = aws_security_group.bench.id
  description                  = "Prometheus /metrics of the DUT and callers"
  ip_protocol                  = "tcp"
  from_port                    = local.metrics_ports.from
  to_port                      = local.metrics_ports.to
  referenced_security_group_id = aws_security_group.bench.id
}

resource "aws_vpc_security_group_ingress_rule" "icmp" {
  security_group_id            = aws_security_group.bench.id
  description                  = "ICMP (reachability checks, PMTU)"
  ip_protocol                  = "icmp"
  from_port                    = -1
  to_port                      = -1
  referenced_security_group_id = aws_security_group.bench.id
}

resource "aws_vpc_security_group_egress_rule" "all" {
  security_group_id = aws_security_group.bench.id
  description       = "All outbound IPv4 (SSM, S3, apt while baking)"
  ip_protocol       = "-1"
  cidr_ipv4         = "0.0.0.0/0"
}

# One cluster placement group per lane: the DUT and its callers share a
# network spine (E74 7.2).
resource "aws_placement_group" "lane" {
  name     = "${var.name_prefix}-lane"
  strategy = "cluster"
}
