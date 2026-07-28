const _: () = assert!(
    cfg!(feature = "s3-archival"),
    "the published standalone engine must compile workflow archival support"
);

#[test]
fn standalone_default_build_includes_s3_archival() {
    let _: fn() -> Option<assay_workflow::archival::ArchivalConfig> =
        assay_workflow::archival::ArchivalConfig::from_env;
}
