mod common;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_s3() {
    let script = r#"
        local s3 = require("assay.s3")
        assert.not_nil(s3)
        assert.not_nil(s3.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_s3_create_bucket() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/test-bucket"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.s3")
        local c = s3.client({{
            endpoint = "{}",
            region = "eu-central-2",
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        }})
        local ok = c:create_bucket("test-bucket")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_s3_list_buckets() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <ListAllMyBucketsResult>
              <Buckets>
                <Bucket><Name>bucket-one</Name><CreationDate>2026-01-01T00:00:00Z</CreationDate></Bucket>
                <Bucket><Name>bucket-two</Name><CreationDate>2026-02-01T00:00:00Z</CreationDate></Bucket>
              </Buckets>
            </ListAllMyBucketsResult>"#,
        ))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.s3")
        local c = s3.client({{
            endpoint = "{}",
            region = "us-east-1",
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
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
async fn test_s3_put_object() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/my-bucket/hello.txt"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.s3")
        local c = s3.client({{
            endpoint = "{}",
            region = "us-east-1",
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        }})
        local ok = c:put_object("my-bucket", "hello.txt", "Hello World!", {{ content_type = "text/plain" }})
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_s3_get_object() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/my-bucket/hello.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Hello World!"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.s3")
        local c = s3.client({{
            endpoint = "{}",
            region = "us-east-1",
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        }})
        local body = c:get_object("my-bucket", "hello.txt")
        assert.eq(body, "Hello World!")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_s3_delete_object() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/my-bucket/hello.txt"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.s3")
        local c = s3.client({{
            endpoint = "{}",
            region = "us-east-1",
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        }})
        local ok = c:delete_object("my-bucket", "hello.txt")
        assert.eq(ok, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_s3_list_objects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/my-bucket"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <ListBucketResult>
              <Name>my-bucket</Name>
              <KeyCount>2</KeyCount>
              <IsTruncated>false</IsTruncated>
              <Contents>
                <Key>file1.txt</Key>
                <Size>1024</Size>
                <LastModified>2026-01-15T10:00:00Z</LastModified>
              </Contents>
              <Contents>
                <Key>file2.txt</Key>
                <Size>2048</Size>
                <LastModified>2026-01-16T12:00:00Z</LastModified>
              </Contents>
            </ListBucketResult>"#,
        ))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.s3")
        local c = s3.client({{
            endpoint = "{}",
            region = "us-east-1",
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        }})
        local result = c:list_objects("my-bucket")
        assert.eq(#result.objects, 2)
        assert.eq(result.objects[1].key, "file1.txt")
        assert.eq(result.objects[1].size, 1024)
        assert.eq(result.objects[2].key, "file2.txt")
        assert.eq(result.is_truncated, false)
        assert.eq(result.key_count, 2)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_s3_bucket_exists() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/existing-bucket"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <ListBucketResult><KeyCount>0</KeyCount><IsTruncated>false</IsTruncated></ListBucketResult>"#,
        ))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local s3 = require("assay.s3")
        local c = s3.client({{
            endpoint = "{}",
            region = "us-east-1",
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        }})
        local exists = c:bucket_exists("existing-bucket")
        assert.eq(exists, true)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
