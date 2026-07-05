mod common;

use common::run_lua;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_aws_s3() {
    let script = r#"
        local s3 = require("assay.aws.s3")
        assert.not_nil(s3)
        assert.not_nil(s3.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_aws_s3_client_requires_credentials() {
    let script = r#"
        local s3 = require("assay.aws.s3")
        local ok = pcall(function()
            s3.client({ region = "us-east-1" })
        end)
        assert.eq(ok, false)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_aws_s3_list_buckets() {
    let server = MockServer::start().await;
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListAllMyBucketsResult>
  <Buckets>
    <Bucket><Name>bucket-one</Name><CreationDate>2026-01-01T00:00:00Z</CreationDate></Bucket>
    <Bucket><Name>bucket-two</Name><CreationDate>2026-02-01T00:00:00Z</CreationDate></Bucket>
  </Buckets>
</ListAllMyBucketsResult>"#;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.aws.s3")
        local c = s3.client({{
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region = "us-east-1",
            endpoint = "{}",
        }})
        local buckets = c:list_buckets()
        assert.eq(#buckets, 2)
        assert.eq(buckets[1].name, "bucket-one")
        assert.eq(buckets[2].name, "bucket-two")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_aws_s3_list_objects() {
    let server = MockServer::start().await;
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult>
  <Name>my-bucket</Name>
  <KeyCount>2</KeyCount>
  <IsTruncated>false</IsTruncated>
  <Contents><Key>a.txt</Key><Size>12</Size><LastModified>2026-01-01T00:00:00Z</LastModified><ETag>"e1"</ETag></Contents>
  <Contents><Key>b.txt</Key><Size>34</Size><LastModified>2026-01-02T00:00:00Z</LastModified><ETag>"e2"</ETag></Contents>
</ListBucketResult>"#;
    Mock::given(method("GET"))
        .and(path("/my-bucket"))
        .and(query_param("list-type", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.aws.s3")
        local c = s3.client({{
            access_key = "AKIA",
            secret_key = "SECRET",
            region = "us-east-1",
            endpoint = "{}",
        }})
        local result = c:list_objects("my-bucket", {{ prefix = "a" }})
        assert.eq(#result.objects, 2)
        assert.eq(result.objects[1].key, "a.txt")
        assert.eq(result.objects[1].size, 12)
        assert.eq(result.key_count, 2)
        assert.eq(result.is_truncated, false)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_aws_s3_head_object() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/my-bucket/hello.txt"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-length", "12"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.aws.s3")
        local c = s3.client({{
            access_key = "AKIA",
            secret_key = "SECRET",
            region = "us-east-1",
            endpoint = "{}",
        }})
        local meta = c:head_object("my-bucket", "hello.txt")
        assert.not_nil(meta)
        assert.eq(meta.status, 200)
        assert.not_nil(meta.headers)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_aws_s3_head_object_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/my-bucket/missing.txt"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.aws.s3")
        local c = s3.client({{
            access_key = "AKIA",
            secret_key = "SECRET",
            region = "us-east-1",
            endpoint = "{}",
        }})
        local meta = c:head_object("my-bucket", "missing.txt")
        assert.eq(meta, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_aws_s3_error_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/my-bucket"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_string(r#"<Error><Code>AccessDenied</Code></Error>"#),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.aws.s3")
        local c = s3.client({{
            access_key = "BADKEY",
            secret_key = "BADSECRET",
            region = "us-east-1",
            endpoint = "{}",
        }})
        local ok, err = pcall(function() c:list_objects("my-bucket") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "AccessDenied")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
