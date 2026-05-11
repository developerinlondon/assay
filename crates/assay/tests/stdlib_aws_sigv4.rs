mod common;

use common::run_lua;

#[tokio::test]
async fn test_require_sigv4() {
    let script = r#"
        local sigv4 = require("assay.aws.sigv4")
        assert.not_nil(sigv4)
        assert.not_nil(sigv4.sign)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_sigv4_sign_produces_authorization_header() {
    let script = r#"
        local sigv4 = require("assay.aws.sigv4")
        local headers = sigv4.sign({
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            service = "ecr",
            region = "us-east-1",
            method = "POST",
            host = "api.ecr.us-east-1.amazonaws.com",
            payload = "{}",
        })
        assert.not_nil(headers.authorization)
        assert.not_nil(headers["x-amz-date"])
        assert.not_nil(headers["x-amz-content-sha256"])
        assert.eq(headers.host, "api.ecr.us-east-1.amazonaws.com")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_sigv4_sign_includes_session_token() {
    let script = r#"
        local sigv4 = require("assay.aws.sigv4")
        local headers = sigv4.sign({
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            session_token = "IQoJb3JpZ2luX2VjEJ",
            service = "ec2",
            region = "eu-west-1",
            method = "GET",
            host = "ec2.eu-west-1.amazonaws.com",
            payload = "",
        })
        assert.eq(headers["x-amz-security-token"], "IQoJb3JpZ2luX2VjEJ")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_sigv4_sign_with_custom_headers() {
    let script = r#"
        local sigv4 = require("assay.aws.sigv4")
        local headers = sigv4.sign({
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            service = "ecr",
            region = "us-east-1",
            method = "POST",
            host = "api.ecr.us-east-1.amazonaws.com",
            payload = "{}",
            headers = {
                ["content-type"] = "application/x-amz-json-1.1",
                ["x-amz-target"] = "AmazonEC2ContainerRegistry_V20150921.GetAuthorizationToken",
            },
        })
        assert.eq(headers["content-type"], "application/x-amz-json-1.1")
        assert.eq(headers["x-amz-target"], "AmazonEC2ContainerRegistry_V20150921.GetAuthorizationToken")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_sigv4_sign_query_params() {
    let script = r#"
        local sigv4 = require("assay.aws.sigv4")
        local headers = sigv4.sign({
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            service = "ecr",
            region = "us-east-1",
            method = "GET",
            host = "api.ecr.us-east-1.amazonaws.com",
            path = "/",
            query = "Action=DescribeRepositories&Version=2015-09-21",
            payload = "",
        })
        assert.not_nil(headers.authorization)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_sigv4_sign_defaults_method_to_get() {
    let script = r#"
        local sigv4 = require("assay.aws.sigv4")
        local headers = sigv4.sign({
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            service = "ecr",
            region = "us-east-1",
            host = "api.ecr.us-east-1.amazonaws.com",
            payload = "",
        })
        -- Default method is GET, path is /
        assert.not_nil(headers.authorization)
    "#;
    run_lua(script).await.unwrap();
}
