# ---------------------------------------------------------------------------
# Evidence bucket: immutable results, logs and bundles. Versioned, private,
# SSE-S3. Only the release-cache/ prefix expires.
# ---------------------------------------------------------------------------

resource "aws_s3_bucket" "evidence" {
  bucket = local.evidence_bucket_name
}

resource "aws_s3_bucket_versioning" "evidence" {
  bucket = aws_s3_bucket.evidence.id

  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_public_access_block" "evidence" {
  bucket = aws_s3_bucket.evidence.id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_server_side_encryption_configuration" "evidence" {
  bucket = aws_s3_bucket.evidence.id

  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "evidence" {
  bucket = aws_s3_bucket.evidence.id

  # Lifecycle rules on a versioned bucket must be applied after versioning.
  depends_on = [aws_s3_bucket_versioning.evidence]

  rule {
    id     = "expire-release-cache"
    status = "Enabled"

    filter {
      prefix = "release-cache/"
    }

    expiration {
      days = 14
    }

    noncurrent_version_expiration {
      noncurrent_days = 30
    }
  }
}

# ---------------------------------------------------------------------------
# Compiler cache bucket: sccache backend. Private, SSE-S3, objects expire
# after 30 days.
# ---------------------------------------------------------------------------

resource "aws_s3_bucket" "cache" {
  bucket = local.cache_bucket_name
}

resource "aws_s3_bucket_public_access_block" "cache" {
  bucket = aws_s3_bucket.cache.id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_server_side_encryption_configuration" "cache" {
  bucket = aws_s3_bucket.cache.id

  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "cache" {
  bucket = aws_s3_bucket.cache.id

  rule {
    id     = "expire-compiler-cache"
    status = "Enabled"

    # Empty filter: the rule applies to every object in the bucket.
    filter {}

    expiration {
      days = 30
    }

    # The bucket is unversioned, but clean up any interrupted multipart uploads.
    abort_incomplete_multipart_upload {
      days_after_initiation = 7
    }
  }
}
