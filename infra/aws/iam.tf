# GitHub OIDC provider. The account already has it; never create a second one.
data "aws_iam_openid_connect_provider" "github" {
  url = "https://token.actions.githubusercontent.com"
}

# ---------------------------------------------------------------------------
# Provisioner role: assumed by the release workflow through OIDC.
# ---------------------------------------------------------------------------

data "aws_iam_policy_document" "provisioner_trust" {
  statement {
    sid     = "GitHubActionsReleaseWorkflow"
    effect  = "Allow"
    actions = ["sts:AssumeRoleWithWebIdentity"]

    principals {
      type        = "Federated"
      identifiers = [data.aws_iam_openid_connect_provider.github.arn]
    }

    condition {
      test     = "StringEquals"
      variable = "token.actions.githubusercontent.com:aud"
      values   = ["sts.amazonaws.com"]
    }

    # Requires the repository's customised sub claim
    # (include_claim_keys = repo, job_workflow_ref); see README.md.
    condition {
      test     = "StringLike"
      variable = "token.actions.githubusercontent.com:sub"
      values = [
        "repo:${var.github_repository}:job_workflow_ref:${var.github_repository}/${var.release_workflow_path}@${var.trusted_refs_glob}",
      ]
    }
  }
}

resource "aws_iam_role" "provisioner" {
  name               = "${var.name_prefix}-gh-provisioner"
  description        = "Assumed by the ${var.github_repository} release qualification workflow to run the ephemeral EC2 worker fleet."
  assume_role_policy = data.aws_iam_policy_document.provisioner_trust.json
}

data "aws_iam_policy_document" "provisioner" {
  # RunInstances: the instance and its volumes must be tagged managed-by at
  # creation (TagSpecifications for both `instance` and `volume`).
  statement {
    sid     = "RunTaggedInstances"
    effect  = "Allow"
    actions = ["ec2:RunInstances"]
    resources = [
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:instance/*",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:volume/*",
    ]

    condition {
      test     = "StringEquals"
      variable = "aws:RequestTag/${local.managed_by_tag_key}"
      values   = [local.managed_by_tag_value]
    }
  }

  # RunInstances also touches these resources, which carry no request tags.
  statement {
    sid     = "RunInstancesSupportingResources"
    effect  = "Allow"
    actions = ["ec2:RunInstances"]
    resources = [
      "arn:${local.partition}:ec2:${var.region}::image/*",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:subnet/*",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:security-group/*",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:network-interface/*",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:key-pair/*",
    ]
  }

  # Tags may only be applied as part of RunInstances, never afterwards.
  statement {
    sid     = "TagOnRunInstances"
    effect  = "Allow"
    actions = ["ec2:CreateTags"]
    resources = [
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:instance/*",
      "arn:${local.partition}:ec2:${var.region}:${local.account_id}:volume/*",
    ]

    condition {
      test     = "StringEquals"
      variable = "ec2:CreateAction"
      values   = ["RunInstances"]
    }
  }

  statement {
    sid    = "DescribeFleet"
    effect = "Allow"
    actions = [
      "ec2:DescribeInstances",
      "ec2:DescribeInstanceTypes",
      "ec2:DescribeImages",
      "ec2:DescribeSubnets",
      "ec2:DescribeSecurityGroups",
      "ec2:DescribeAvailabilityZones",
    ]
    resources = ["*"]
  }

  statement {
    sid    = "StopTerminateManagedInstances"
    effect = "Allow"
    actions = [
      "ec2:StopInstances",
      "ec2:TerminateInstances",
    ]
    resources = ["arn:${local.partition}:ec2:${var.region}:${local.account_id}:instance/*"]

    condition {
      test     = "StringEquals"
      variable = "aws:ResourceTag/${local.managed_by_tag_key}"
      values   = [local.managed_by_tag_value]
    }
  }

  statement {
    sid       = "PassRunnerRoleToEc2"
    effect    = "Allow"
    actions   = ["iam:PassRole"]
    resources = [aws_iam_role.runner.arn]

    condition {
      test     = "StringEquals"
      variable = "iam:PassedToService"
      values   = ["ec2.amazonaws.com"]
    }
  }

  statement {
    sid       = "ReadUbuntuAmiParameter"
    effect    = "Allow"
    actions   = ["ssm:GetParameter"]
    resources = ["arn:${local.partition}:ssm:${var.region}::parameter/aws/service/canonical/ubuntu/*"]
  }

  statement {
    sid    = "ReadServiceQuotas"
    effect = "Allow"
    actions = [
      "servicequotas:GetServiceQuota",
      "servicequotas:GetAWSDefaultServiceQuota",
    ]
    resources = ["*"]
  }

  statement {
    sid     = "ListBuckets"
    effect  = "Allow"
    actions = ["s3:ListBucket"]
    resources = [
      aws_s3_bucket.evidence.arn,
      aws_s3_bucket.cache.arn,
    ]
  }

  statement {
    sid     = "ReadBucketObjects"
    effect  = "Allow"
    actions = ["s3:GetObject"]
    resources = [
      "${aws_s3_bucket.evidence.arn}/*",
      "${aws_s3_bucket.cache.arn}/*",
    ]
  }
}

resource "aws_iam_role_policy" "provisioner" {
  name   = "${var.name_prefix}-gh-provisioner"
  role   = aws_iam_role.provisioner.id
  policy = data.aws_iam_policy_document.provisioner.json
}

# ---------------------------------------------------------------------------
# Runner role and instance profile: attached to every worker instance.
# ---------------------------------------------------------------------------

data "aws_iam_policy_document" "runner_trust" {
  statement {
    effect  = "Allow"
    actions = ["sts:AssumeRole"]

    principals {
      type        = "Service"
      identifiers = ["ec2.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "runner" {
  name               = "${var.name_prefix}-runner"
  description        = "Instance role for ephemeral release workers: evidence upload and compiler cache only."
  assume_role_policy = data.aws_iam_policy_document.runner_trust.json
}

data "aws_iam_policy_document" "runner" {
  statement {
    sid     = "ListBuckets"
    effect  = "Allow"
    actions = ["s3:ListBucket"]
    resources = [
      aws_s3_bucket.evidence.arn,
      aws_s3_bucket.cache.arn,
    ]
  }

  statement {
    sid    = "EvidenceObjects"
    effect = "Allow"
    actions = [
      "s3:PutObject",
      "s3:GetObject",
    ]
    resources = ["${aws_s3_bucket.evidence.arn}/*"]
  }

  statement {
    sid    = "CacheObjects"
    effect = "Allow"
    actions = [
      "s3:PutObject",
      "s3:GetObject",
      "s3:DeleteObject",
    ]
    resources = ["${aws_s3_bucket.cache.arn}/*"]
  }
}

resource "aws_iam_role_policy" "runner" {
  name   = "${var.name_prefix}-runner"
  role   = aws_iam_role.runner.id
  policy = data.aws_iam_policy_document.runner.json
}

resource "aws_iam_instance_profile" "runner" {
  name = "${var.name_prefix}-runner"
  role = aws_iam_role.runner.name
}
