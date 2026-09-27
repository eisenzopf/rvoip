# Janitor: a scheduled Lambda that terminates managed-by=github-actions
# instances whose rvoip-expires-at tag is in the past. Third and last
# cleanup layer after worker self-termination and the controller's
# always() terminate.

locals {
  janitor_name = "${var.name_prefix}-janitor"
  janitor_schedule = (
    var.worker_expiry_check_minutes == 1
    ? "rate(1 minute)"
    : "rate(${var.worker_expiry_check_minutes} minutes)"
  )
}

data "archive_file" "janitor" {
  type        = "zip"
  source_file = "${path.module}/lambda/janitor.py"
  output_path = "${path.module}/.build/janitor.zip"
}

data "aws_iam_policy_document" "janitor_trust" {
  statement {
    effect  = "Allow"
    actions = ["sts:AssumeRole"]

    principals {
      type        = "Service"
      identifiers = ["lambda.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "janitor" {
  name               = local.janitor_name
  description        = "Terminates expired rvoip release workers."
  assume_role_policy = data.aws_iam_policy_document.janitor_trust.json
}

resource "aws_cloudwatch_log_group" "janitor" {
  name              = "/aws/lambda/${local.janitor_name}"
  retention_in_days = 90
}

data "aws_iam_policy_document" "janitor" {
  statement {
    sid       = "DescribeInstances"
    effect    = "Allow"
    actions   = ["ec2:DescribeInstances"]
    resources = ["*"]
  }

  statement {
    sid       = "TerminateManagedInstances"
    effect    = "Allow"
    actions   = ["ec2:TerminateInstances"]
    resources = ["arn:${local.partition}:ec2:${var.region}:${local.account_id}:instance/*"]

    condition {
      test     = "StringEquals"
      variable = "aws:ResourceTag/${local.managed_by_tag_key}"
      values   = [local.managed_by_tag_value]
    }
  }

  statement {
    sid    = "WriteLogs"
    effect = "Allow"
    actions = [
      "logs:CreateLogStream",
      "logs:PutLogEvents",
    ]
    resources = ["${aws_cloudwatch_log_group.janitor.arn}:*"]
  }
}

resource "aws_iam_role_policy" "janitor" {
  name   = local.janitor_name
  role   = aws_iam_role.janitor.id
  policy = data.aws_iam_policy_document.janitor.json
}

resource "aws_lambda_function" "janitor" {
  function_name = local.janitor_name
  description   = "Terminates rvoip release workers whose ${local.expires_at_tag_key} tag is in the past."
  role          = aws_iam_role.janitor.arn

  filename         = data.archive_file.janitor.output_path
  source_code_hash = data.archive_file.janitor.output_base64sha256
  handler          = "janitor.handler"
  runtime          = "python3.12"
  architectures    = ["arm64"]
  timeout          = 60
  memory_size      = 128

  environment {
    variables = {
      MANAGED_BY_TAG_KEY   = local.managed_by_tag_key
      MANAGED_BY_TAG_VALUE = local.managed_by_tag_value
      EXPIRES_AT_TAG_KEY   = local.expires_at_tag_key
    }
  }

  logging_config {
    log_format = "Text"
    log_group  = aws_cloudwatch_log_group.janitor.name
  }

  depends_on = [aws_iam_role_policy.janitor]
}

resource "aws_cloudwatch_event_rule" "janitor" {
  name                = local.janitor_name
  description         = "Sweep expired rvoip release workers."
  schedule_expression = local.janitor_schedule
}

resource "aws_cloudwatch_event_target" "janitor" {
  rule      = aws_cloudwatch_event_rule.janitor.name
  target_id = "lambda"
  arn       = aws_lambda_function.janitor.arn
}

resource "aws_lambda_permission" "janitor_events" {
  statement_id  = "AllowEventBridgeInvoke"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.janitor.function_name
  principal     = "events.amazonaws.com"
  source_arn    = aws_cloudwatch_event_rule.janitor.arn
}
