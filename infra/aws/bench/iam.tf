# ---------------------------------------------------------------------------
# Bench host role: on the DUT and caller instances. SSM agent plus evidence
# read/write under bench/ only.
# ---------------------------------------------------------------------------

data "aws_iam_policy_document" "host_trust" {
  statement {
    effect  = "Allow"
    actions = ["sts:AssumeRole"]

    principals {
      type        = "Service"
      identifiers = ["ec2.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "host" {
  name               = "${var.name_prefix}-host"
  description        = "rvoip bench lane hosts: SSM and bench/ evidence in the release evidence bucket."
  assume_role_policy = data.aws_iam_policy_document.host_trust.json
}

resource "aws_iam_role_policy_attachment" "host_ssm" {
  role       = aws_iam_role.host.name
  policy_arn = "arn:${local.partition}:iam::aws:policy/AmazonSSMManagedInstanceCore"
}

data "aws_iam_policy_document" "host" {
  statement {
    sid       = "ListBenchPrefix"
    effect    = "Allow"
    actions   = ["s3:ListBucket"]
    resources = [data.aws_s3_bucket.evidence.arn]

    condition {
      test     = "StringLike"
      variable = "s3:prefix"
      values   = ["${local.evidence_prefix}*"]
    }
  }

  statement {
    sid       = "BenchObjects"
    effect    = "Allow"
    actions   = ["s3:GetObject", "s3:PutObject"]
    resources = ["${data.aws_s3_bucket.evidence.arn}/${local.evidence_prefix}*"]
  }
}

resource "aws_iam_role_policy" "host" {
  name   = "${var.name_prefix}-host"
  role   = aws_iam_role.host.id
  policy = data.aws_iam_policy_document.host.json
}

resource "aws_iam_instance_profile" "host" {
  name = "${var.name_prefix}-host"
  role = aws_iam_role.host.name
}

# ---------------------------------------------------------------------------
# Orchestrator policy: what scripts/bench/run_lane.py needs. A managed policy,
# not attached here: attach it to whatever runs the lane (the release
# executor's role, or an operator's role).
# ---------------------------------------------------------------------------

data "aws_iam_policy_document" "orchestrator" {
  statement {
    sid     = "LaunchFromBenchTemplates"
    effect  = "Allow"
    actions = ["ec2:RunInstances"]
    resources = [
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:instance/*",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:volume/*",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:network-interface/*",
    ]

    condition {
      test     = "StringEquals"
      variable = "aws:RequestTag/rvoip-bench"
      values   = ["true"]
    }
  }

  statement {
    sid     = "LaunchSupportingResources"
    effect  = "Allow"
    actions = ["ec2:RunInstances"]
    resources = concat(
      ["arn:${local.partition}:ec2:${var.region}::image/*"],
      # Any release public subnet: run_lane.py --az moves a lane off the template AZ.
      [for id in sort(data.aws_subnets.release_public.ids) : "arn:${local.partition}:ec2:${var.region}:${local.account_id}:subnet/${id}"],
      [
        "arn:${local.partition}:ec2:${var.region}:${local.account_id}:security-group/${aws_security_group.bench.id}",
        "arn:${local.partition}:ec2:${var.region}:${local.account_id}:launch-template/${aws_launch_template.dut.id}",
        "arn:${local.partition}:ec2:${var.region}:${local.account_id}:launch-template/${aws_launch_template.caller.id}",
        aws_placement_group.lane.arn,
      ],
    )
  }

  # A targeted launch names the lane's capacity reservation: only bench reservations.
  statement {
    sid       = "LaunchIntoBenchReservations"
    effect    = "Allow"
    actions   = ["ec2:RunInstances"]
    resources = ["arn:${local.partition}:ec2:${var.region}:${local.account_id}:capacity-reservation/*"]

    condition {
      test     = "StringEquals"
      variable = "aws:ResourceTag/rvoip-bench"
      values   = ["true"]
    }
  }

  # minor/full reserve the lane (1 DUT + N callers) in its AZ and placement group.
  statement {
    sid       = "ReserveBenchCapacity"
    effect    = "Allow"
    actions   = ["ec2:CreateCapacityReservation"]
    resources = ["arn:${local.partition}:ec2:${var.region}:${local.account_id}:capacity-reservation/*"]

    condition {
      test     = "StringEquals"
      variable = "aws:RequestTag/rvoip-bench"
      values   = ["true"]
    }
  }

  statement {
    sid       = "ReserveInTheLanePlacementGroup"
    effect    = "Allow"
    actions   = ["ec2:CreateCapacityReservation"]
    resources = [aws_placement_group.lane.arn]
  }

  # Hold across a --max-wall stop (shorter end date), adopt on resume (longer), cancel on exit or in
  # the workflow's cleanup job: bench reservations only.
  statement {
    sid       = "ManageBenchReservations"
    effect    = "Allow"
    actions   = ["ec2:ModifyCapacityReservation", "ec2:CancelCapacityReservation"]
    resources = ["arn:${local.partition}:ec2:${var.region}:${local.account_id}:capacity-reservation/*"]

    condition {
      test     = "StringEquals"
      variable = "aws:ResourceTag/rvoip-bench"
      values   = ["true"]
    }
  }

  statement {
    sid       = "TagOnLaunch"
    effect    = "Allow"
    actions   = ["ec2:CreateTags"]
    resources = ["arn:${local.partition}:ec2:${var.region}:${local.account_id}:*/*"]

    condition {
      test     = "StringEquals"
      variable = "ec2:CreateAction"
      values   = ["RunInstances", "CreateImage", "CreateCapacityReservation"]
    }
  }

  # The held-for-resume marker on an existing bench reservation, and nothing else.
  statement {
    sid       = "MarkHeldReservations"
    effect    = "Allow"
    actions   = ["ec2:CreateTags"]
    resources = ["arn:${local.partition}:ec2:${var.region}:${local.account_id}:capacity-reservation/*"]

    condition {
      test     = "StringEquals"
      variable = "aws:ResourceTag/rvoip-bench"
      values   = ["true"]
    }

    condition {
      test     = "ForAllValues:StringEquals"
      variable = "aws:TagKeys"
      values   = ["rvoip-held-for-resume"]
    }
  }

  statement {
    sid       = "TerminateBenchInstances"
    effect    = "Allow"
    actions   = ["ec2:TerminateInstances", "ec2:StopInstances"]
    resources = ["arn:${local.partition}:ec2:${var.region}:${local.account_id}:instance/*"]

    condition {
      test     = "StringEquals"
      variable = "aws:ResourceTag/rvoip-bench"
      values   = ["true"]
    }
  }

  statement {
    sid    = "BakeImages"
    effect = "Allow"
    actions = [
      "ec2:CreateImage",
    ]
    resources = ["*"]
  }

  statement {
    sid    = "Describe"
    effect = "Allow"
    actions = [
      "ec2:DescribeInstances",
      "ec2:DescribeInstanceStatus",
      "ec2:DescribeInstanceTypes",
      "ec2:DescribeImages",
      "ec2:DescribeLaunchTemplates",
      "ec2:DescribeLaunchTemplateVersions",
      "ec2:DescribeSubnets",
      "ec2:DescribeNetworkInterfaces",
      "ec2:DescribePlacementGroups",
      "ec2:DescribeCapacityReservations",
      "servicequotas:GetServiceQuota",
      "ssm:DescribeInstanceInformation",
      "ssm:GetCommandInvocation",
      "ssm:ListCommandInvocations",
      "ssm:CancelCommand",
    ]
    resources = ["*"]
  }

  statement {
    sid       = "RunCommandOnBenchHosts"
    effect    = "Allow"
    actions   = ["ssm:SendCommand"]
    resources = ["arn:${local.partition}:ec2:${var.region}:${local.account_id}:instance/*"]

    condition {
      test     = "StringEquals"
      variable = "ssm:resourceTag/rvoip-bench"
      values   = ["true"]
    }
  }

  statement {
    sid       = "RunShellScriptDocument"
    effect    = "Allow"
    actions   = ["ssm:SendCommand"]
    resources = ["arn:${local.partition}:ssm:${var.region}::document/AWS-RunShellScript"]
  }

  statement {
    sid       = "PassHostRole"
    effect    = "Allow"
    actions   = ["iam:PassRole"]
    resources = [aws_iam_role.host.arn]

    condition {
      test     = "StringEquals"
      variable = "iam:PassedToService"
      values   = ["ec2.amazonaws.com"]
    }
  }

  statement {
    sid       = "ListBenchPrefix"
    effect    = "Allow"
    actions   = ["s3:ListBucket"]
    resources = [data.aws_s3_bucket.evidence.arn]

    condition {
      test     = "StringLike"
      variable = "s3:prefix"
      values   = ["${local.evidence_prefix}*"]
    }
  }

  statement {
    sid       = "BenchObjects"
    effect    = "Allow"
    actions   = ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"]
    resources = ["${data.aws_s3_bucket.evidence.arn}/${local.evidence_prefix}*"]
  }
}

resource "aws_iam_policy" "orchestrator" {
  name        = "${var.name_prefix}-orchestrator"
  description = "Run one rvoip bench lane (scripts/bench/run_lane.py): launch from the bench templates, SSM to bench hosts, bench/ evidence."
  policy      = data.aws_iam_policy_document.orchestrator.json
}

# The Release qualification workflow's bench legs, gate-bench and its bench cleanup job run
# run_lane.py (or cancel this run's reservations) as the provisioner role.
data "aws_iam_role" "release_provisioner" {
  count = var.release_provisioner_role_name == "" ? 0 : 1
  name  = var.release_provisioner_role_name
}

resource "aws_iam_role_policy_attachment" "release_provisioner_orchestrator" {
  count      = var.release_provisioner_role_name == "" ? 0 : 1
  role       = data.aws_iam_role.release_provisioner[0].name
  policy_arn = aws_iam_policy.orchestrator.arn
}
