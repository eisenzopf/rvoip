data "aws_ec2_instance_type" "dut" {
  instance_type = var.dut_instance_type
}

data "aws_ec2_instance_type" "caller" {
  instance_type = var.caller_instance_type
}

# Running On-Demand Standard (A, C, D, H, I, M, R, T, Z) instances, in vCPUs.
data "aws_servicequotas_service_quota" "standard_vcpus" {
  service_code = "ec2"
  quota_code   = "L-1216C47A"
}

locals {
  lane_vcpus = data.aws_ec2_instance_type.dut.default_vcpus + var.caller_count * data.aws_ec2_instance_type.caller.default_vcpus
}

check "lane_fits_vcpu_quota" {
  assert {
    condition     = local.lane_vcpus + var.qualification_vcpus <= data.aws_servicequotas_service_quota.standard_vcpus.value
    error_message = "A bench lane (${local.lane_vcpus} vCPU) plus the qualification fleet (${var.qualification_vcpus} vCPU) exceeds the on-demand vCPU quota (${data.aws_servicequotas_service_quota.standard_vcpus.value})."
  }
}

check "az_offers_lane_types" {
  assert {
    condition     = length(distinct(data.aws_ec2_instance_type_offerings.lane.instance_types)) == length(distinct([var.dut_instance_type, var.caller_instance_type]))
    error_message = "${var.availability_zone} does not offer both ${var.dut_instance_type} and ${var.caller_instance_type}."
  }
}
