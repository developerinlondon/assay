mod common;

use common::run_lua;

// The EKS token is a presigned sts:GetCallerIdentity URL, base64url-encoded
// with the k8s-aws-v1. prefix — minted entirely in-process (no aws CLI).
#[tokio::test]
async fn test_eks_get_token_shape() {
    let script = r#"
        local eks = require("assay.aws.eks")
        local token = eks.get_token("my-cluster", {
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region = "us-east-1",
            time = 1720000000,
        })
        assert.eq(token:sub(1, 11), "k8s-aws-v1.")

        -- Decode the base64url payload back to the presigned URL and verify it.
        local b64 = token:sub(12):gsub("-", "+"):gsub("_", "/")
        local pad = #b64 % 4
        if pad == 2 then b64 = b64 .. "==" elseif pad == 3 then b64 = b64 .. "=" end
        local url = base64.decode(b64)
        assert.contains(url, "https://sts.us-east-1.amazonaws.com/?")
        assert.contains(url, "Action=GetCallerIdentity")
        assert.contains(url, "Version=2011-06-15")
        assert.contains(url, "X-Amz-SignedHeaders=host%3Bx-k8s-aws-id")
        assert.contains(url, "X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20240703%2Fus-east-1%2Fsts%2Faws4_request")
        assert.not_nil(url:match("X%-Amz%-Signature=%x+"))
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_eks_get_token_session_token_included() {
    let script = r#"
        local eks = require("assay.aws.eks")
        local token = eks.get_token("qa-workload", {
            credentials = {
                access_key = "ASIATEMP",
                secret_key = "tempsecret",
                session_token = "temptoken",
            },
            region = "eu-west-1",
            time = 1720000000,
        })
        local b64 = token:sub(12):gsub("-", "+"):gsub("_", "/")
        local pad = #b64 % 4
        if pad == 2 then b64 = b64 .. "==" elseif pad == 3 then b64 = b64 .. "=" end
        local url = base64.decode(b64)
        assert.contains(url, "sts.eu-west-1.amazonaws.com")
        assert.contains(url, "X-Amz-Security-Token=temptoken")
    "#;
    run_lua(script).await.unwrap();
}
