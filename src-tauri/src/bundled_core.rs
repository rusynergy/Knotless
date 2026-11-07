use crate::*;
#[derive(Deserialize)]struct Manifest{version:String,architecture:String,files:std::collections::BTreeMap<String,String>}
pub fn install(root:&Path,resources:&Path)->Result<Option<String>>{
    let source=resources.join("resources").join("xray");
    let path=source.join("manifest.json");
    if !path.exists(){return Ok(None);} // Development without packaged resources.
    let manifest:Manifest=serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)?;
    valid_version(&manifest.version)?;
    let arch=if cfg!(target_arch="aarch64"){ "arm64" }else{ "x64" };
    if manifest.architecture!=arch{return Err("Bundled Xray architecture mismatch".into());}
    let target=root.join(CORE_DIRECTORY).join(&manifest.version);
    if target.join(CORE_EXE).is_file(){return Ok(Some(manifest.version));}
    let staging=root.join(CORE_DIRECTORY).join(format!(".bundled-{}",uuid::Uuid::new_v4()));
    fs::create_dir_all(&staging).map_err(err)?;
    let result=(||{
        for name in [CORE_EXE,"geoip.dat","geosite.dat","LICENSE"]{
            let expected=manifest.files.get(name).ok_or("Missing bundled Xray checksum")?;
            let bytes=fs::read(source.join(name)).map_err(err)?;
            if !format!("{:x}",Sha256::digest(&bytes)).eq_ignore_ascii_case(expected){return Err("Bundled Xray checksum mismatch".into());}
            fs::write(staging.join(name),bytes).map_err(err)?;
        }
        #[cfg(target_os="macos")] {use std::os::unix::fs::PermissionsExt; fs::set_permissions(staging.join(CORE_EXE),fs::Permissions::from_mode(0o755)).map_err(err)?;}
        fs::rename(&staging,&target).map_err(err)?;Ok(Some(manifest.version))
    })();
    if staging.exists(){for name in [CORE_EXE,"geoip.dat","geosite.dat","LICENSE"]{let _=fs::remove_file(staging.join(name));}let _=fs::remove_dir(staging);}
    result
}
#[cfg(test)]mod tests{use super::*;
 #[test]fn installs_offline_and_preserves_existing_core(){
    let root=std::env::temp_dir().join(format!("knotless-bundle-test-{}",uuid::Uuid::new_v4()));let source=root.join("resources/xray");fs::create_dir_all(&source).unwrap();let mut files=serde_json::Map::new();
    for name in [CORE_EXE,"geoip.dat","geosite.dat","LICENSE"]{fs::write(source.join(name),b"fixture").unwrap();files.insert(name.into(),json!(format!("{:x}",Sha256::digest(b"fixture"))));}
    fs::write(source.join("manifest.json"),serde_json::to_vec(&json!({"version":"v26.10.31","architecture":if cfg!(target_arch="aarch64"){"arm64"}else{"x64"},"files":files})).unwrap()).unwrap();
    assert_eq!(install(&root,&root).unwrap(),Some("v26.10.31".into()));let target=root.join(CORE_DIRECTORY).join("v26.10.31");fs::write(target.join(CORE_EXE),b"existing").unwrap();install(&root,&root).unwrap();assert_eq!(fs::read(target.join(CORE_EXE)).unwrap(),b"existing");
    let mut manifest:serde_json::Value=serde_json::from_slice(&fs::read(source.join("manifest.json")).unwrap()).unwrap();
    manifest["version"]=json!("v26.10.32");fs::write(source.join("manifest.json"),serde_json::to_vec(&manifest).unwrap()).unwrap();
    fs::write(source.join("geosite.dat"),b"tampered").unwrap();
    assert!(install(&root,&root).unwrap_err().contains("checksum"));
    assert!(!root.join(CORE_DIRECTORY).join("v26.10.32").exists());
    let checked=root.canonicalize().unwrap();
    assert!(checked.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    assert!(checked.file_name().unwrap().to_string_lossy().starts_with("knotless-bundle-test-"));
    fs::remove_dir_all(checked).unwrap();
 }
}
