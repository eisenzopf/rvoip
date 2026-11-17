locals {
  # run_lane.py adds rvoip-run-id, rvoip-role and rvoip-expires-at at launch.
  instance_tags = {
    "rvoip-bench"              = "true"
    (local.managed_by_tag_key) = local.managed_by_tag_value
  }
}

resource "aws_launch_template" "dut" {
  name                                 = "${var.name_prefix}-dut"
  description                          = "rvoip bench DUT (rvoip-bench-uas), ${var.dut_instance_type}"
  image_id                             = local.ami_id
  instance_type                        = var.dut_instance_type
  instance_initiated_shutdown_behavior = "terminate"
  update_default_version               = true

  iam_instance_profile {
    arn = aws_iam_instance_profile.host.arn
  }

  placement {
    group_name        = aws_placement_group.lane.name
    availability_zone = var.availability_zone
  }

  network_interfaces {
    device_index                = 0
    subnet_id                   = data.aws_subnet.lane.id
    security_groups             = [aws_security_group.bench.id]
    associate_public_ip_address = true
    delete_on_termination       = true
  }

  block_device_mappings {
    device_name = "/dev/sda1"

    ebs {
      volume_type           = "gp3"
      volume_size           = var.dut_root_gb
      iops                  = 6000
      throughput            = 500
      delete_on_termination = true
      encrypted             = true
    }
  }

  metadata_options {
    http_endpoint               = "enabled"
    http_tokens                 = "required"
    http_put_response_hop_limit = 1
    instance_metadata_tags      = "enabled"
  }

  tag_specifications {
    resource_type = "instance"
    tags          = merge(local.instance_tags, { Name = "${var.name_prefix}-dut", "rvoip-role" = "dut" })
  }

  tag_specifications {
    resource_type = "volume"
    tags          = merge(local.instance_tags, { Name = "${var.name_prefix}-dut" })
  }
}

resource "aws_launch_template" "caller" {
  name                                 = "${var.name_prefix}-caller"
  description                          = "rvoip bench caller (rvoip-loadgen / ea-uas), ${var.caller_instance_type}, ${var.caller_sources} source IPs"
  image_id                             = local.ami_id
  instance_type                        = var.caller_instance_type
  instance_initiated_shutdown_behavior = "terminate"
  update_default_version               = true

  iam_instance_profile {
    arn = aws_iam_instance_profile.host.arn
  }

  placement {
    group_name        = aws_placement_group.lane.name
    availability_zone = var.availability_zone
  }

  network_interfaces {
    device_index                = 0
    subnet_id                   = data.aws_subnet.lane.id
    security_groups             = [aws_security_group.bench.id]
    associate_public_ip_address = true
    delete_on_termination       = true
    ipv4_address_count          = var.caller_sources - 1 > 0 ? var.caller_sources - 1 : null
  }

  block_device_mappings {
    device_name = "/dev/sda1"

    ebs {
      volume_type           = "gp3"
      volume_size           = var.caller_root_gb
      iops                  = 6000
      throughput            = 500
      delete_on_termination = true
      encrypted             = true
    }
  }

  metadata_options {
    http_endpoint               = "enabled"
    http_tokens                 = "required"
    http_put_response_hop_limit = 1
    instance_metadata_tags      = "enabled"
  }

  tag_specifications {
    resource_type = "instance"
    tags          = merge(local.instance_tags, { Name = "${var.name_prefix}-caller", "rvoip-role" = "caller" })
  }

  tag_specifications {
    resource_type = "volume"
    tags          = merge(local.instance_tags, { Name = "${var.name_prefix}-caller" })
  }
}
