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
    resources = [
      "arn:${local.partition}:ec2:${var.region}::image/*",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:subnet/${data.aws_subnet.lane.id}",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:security-group/${aws_security_group.bench.id}",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:launch-template/${aws_launch_template.dut.id}",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:launch-template/${aws_launch_template.caller.id}",
      aws_placement_group.lane.arn,
    ]
  }

  statement {
    sid       = "TagOnLaunch"
    effect    = "Allow"
    actions   = ["ec2:CreateTags"]
    resources = ["arn:${local.partition}:ec2:${var.region}:${local.account_id}:*/*"]

    condition {
      test     = "StringEquals"
      variable = "ec2:CreateAction"
      values   = ["RunInstances", "CreateImage"]
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
      "servicequotas:GetServiceQuota",
      "ssm:DescribeInstanceInformation",
      "ssm:GetCommandInvocation",
      "ssm:ListCommandInvocations",
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
