mod common;

use common::run_lua;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_require_ec2() {
    let script = r#"
        local ec2 = require("assay.aws.ec2")
        assert.not_nil(ec2)
        assert.not_nil(ec2.client)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_ec2_client_requires_credentials() {
    let script = r#"
        local ec2 = require("assay.aws.ec2")
        local ok = pcall(function()
            ec2.client({ region = "us-east-1" })
        end)
        assert.eq(ok, false)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_ec2_describe_instances() {
    let server = MockServer::start().await;
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <reservationSet>
    <item>
      <reservationId>r-1</reservationId>
      <ownerId>111122223333</ownerId>
      <instancesSet>
        <item>
          <instanceId>i-aaa</instanceId>
          <instanceType>t3.micro</instanceType>
          <instanceState><code>16</code><name>running</name></instanceState>
          <privateIpAddress>10.0.0.10</privateIpAddress>
          <ipAddress>52.1.2.3</ipAddress>
          <placement><availabilityZone>us-east-1a</availabilityZone></placement>
          <networkInterfaceSet>
            <item>
              <networkInterfaceId>eni-1</networkInterfaceId>
              <privateIpAddress>10.0.0.10</privateIpAddress>
              <attachment><instanceId>i-aaa</instanceId></attachment>
            </item>
          </networkInterfaceSet>
        </item>
        <item>
          <instanceId>i-bbb</instanceId>
          <instanceType>t3.small</instanceType>
          <instanceState><code>80</code><name>stopped</name></instanceState>
          <privateIpAddress>10.0.0.11</privateIpAddress>
          <placement><availabilityZone>us-east-1b</availabilityZone></placement>
        </item>
      </instancesSet>
    </item>
  </reservationSet>
</DescribeInstancesResponse>"#;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(query_param("Action", "DescribeInstances"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local ec2 = require("assay.aws.ec2")
        local c = ec2.client({{
            access_key = "AKIAIOSFODNN7EXAMPLE",
            secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region = "us-east-1",
            endpoint = "{}",
        }})
        local instances = c:describe_instances()
        assert.eq(#instances, 2)
        assert.eq(instances[1].instance_id, "i-aaa")
        assert.eq(instances[1].instance_type, "t3.micro")
        assert.eq(instances[1].state, "running")
        assert.eq(instances[1].private_ip, "10.0.0.10")
        assert.eq(instances[1].public_ip, "52.1.2.3")
        assert.eq(instances[1].availability_zone, "us-east-1a")
        assert.eq(instances[2].instance_id, "i-bbb")
        assert.eq(instances[2].state, "stopped")
        assert.eq(instances[2].public_ip, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_ec2_describe_instances_empty() {
    let server = MockServer::start().await;
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <reservationSet/>
</DescribeInstancesResponse>"#;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(query_param("Action", "DescribeInstances"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local ec2 = require("assay.aws.ec2")
        local c = ec2.client({{
            access_key = "AKIA",
            secret_key = "SECRET",
            region = "us-east-1",
            endpoint = "{}",
        }})
        local instances = c:describe_instances()
        assert.eq(#instances, 0)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_ec2_describe_volumes() {
    let server = MockServer::start().await;
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<DescribeVolumesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <volumeSet>
    <item>
      <volumeId>vol-1</volumeId>
      <size>100</size>
      <status>in-use</status>
      <availabilityZone>us-east-1a</availabilityZone>
      <attachmentSet>
        <item>
          <volumeId>vol-1</volumeId>
          <instanceId>i-aaa</instanceId>
          <status>attached</status>
        </item>
      </attachmentSet>
      <volumeType>gp3</volumeType>
    </item>
    <item>
      <volumeId>vol-2</volumeId>
      <size>8</size>
      <status>available</status>
      <availabilityZone>us-east-1b</availabilityZone>
      <volumeType>gp2</volumeType>
    </item>
  </volumeSet>
</DescribeVolumesResponse>"#;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(query_param("Action", "DescribeVolumes"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local ec2 = require("assay.aws.ec2")
        local c = ec2.client({{
            access_key = "AKIA",
            secret_key = "SECRET",
            region = "us-east-1",
            endpoint = "{}",
        }})
        local volumes = c:describe_volumes()
        assert.eq(#volumes, 2)
        assert.eq(volumes[1].volume_id, "vol-1")
        assert.eq(volumes[1].size, 100)
        assert.eq(volumes[1].state, "in-use")
        assert.eq(volumes[1].volume_type, "gp3")
        assert.eq(volumes[2].volume_id, "vol-2")
        assert.eq(volumes[2].state, "available")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_ec2_describe_security_groups() {
    let server = MockServer::start().await;
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<DescribeSecurityGroupsResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <securityGroupInfo>
    <item>
      <ownerId>111122223333</ownerId>
      <groupId>sg-1</groupId>
      <groupName>web</groupName>
      <groupDescription>web tier</groupDescription>
      <vpcId>vpc-1</vpcId>
      <ipPermissions>
        <item>
          <ipProtocol>tcp</ipProtocol>
          <fromPort>443</fromPort>
          <toPort>443</toPort>
          <groups>
            <item><groupId>sg-2</groupId><userId>111122223333</userId></item>
          </groups>
        </item>
      </ipPermissions>
    </item>
    <item>
      <ownerId>111122223333</ownerId>
      <groupId>sg-2</groupId>
      <groupName>db</groupName>
      <groupDescription>db tier</groupDescription>
      <vpcId>vpc-1</vpcId>
    </item>
  </securityGroupInfo>
</DescribeSecurityGroupsResponse>"#;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(query_param("Action", "DescribeSecurityGroups"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local ec2 = require("assay.aws.ec2")
        local c = ec2.client({{
            access_key = "AKIA",
            secret_key = "SECRET",
            region = "us-east-1",
            endpoint = "{}",
        }})
        local groups = c:describe_security_groups()
        assert.eq(#groups, 2)
        assert.eq(groups[1].group_id, "sg-1")
        assert.eq(groups[1].group_name, "web")
        assert.eq(groups[1].description, "web tier")
        assert.eq(groups[1].vpc_id, "vpc-1")
        assert.eq(groups[2].group_id, "sg-2")
        assert.eq(groups[2].group_name, "db")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_ec2_error_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .and(query_param("Action", "DescribeInstances"))
        .respond_with(ResponseTemplate::new(403).set_body_string(
            r#"<Response><Errors><Error><Code>AuthFailure</Code></Error></Errors></Response>"#,
        ))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local ec2 = require("assay.aws.ec2")
        local c = ec2.client({{
            access_key = "BADKEY",
            secret_key = "BADSECRET",
            region = "us-east-1",
            endpoint = "{}",
        }})
        local ok, err = pcall(function() c:describe_instances() end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "AuthFailure")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}
