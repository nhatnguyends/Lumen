//! DDC/CI input switching; Windows handles are always released on the worker.
use super::backend::{Device, Display};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    mem::{size_of, zeroed},
    ptr::null_mut,
    time::{Duration, Instant},
};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*};
#[repr(C)]
struct Physical {
    handle: HANDLE,
    name: [u16; 128],
}
#[link(name = "dxva2")]
extern "system" {
    fn GetNumberOfPhysicalMonitorsFromHMONITOR(h: HMONITOR, n: *mut u32) -> i32;
    fn GetPhysicalMonitorsFromHMONITOR(h: HMONITOR, n: u32, p: *mut Physical) -> i32;
    fn DestroyPhysicalMonitors(n: u32, p: *mut Physical) -> i32;
    fn GetVCPFeatureAndVCPFeatureReply(
        h: HANDLE,
        code: u8,
        ty: *mut u32,
        current: *mut u32,
        max: *mut u32,
    ) -> i32;
    fn SetVCPFeature(h: HANDLE, code: u8, value: u32) -> i32;
    fn GetCapabilitiesStringLength(h: HANDLE, len: *mut u32) -> i32;
    fn CapabilitiesRequestAndCapabilitiesReply(h: HANDLE, text: *mut u8, len: u32) -> i32;
}
struct Handles(Vec<Physical>);
impl Drop for Handles {
    fn drop(&mut self) {
        unsafe {
            DestroyPhysicalMonitors(self.0.len() as u32, self.0.as_mut_ptr());
        }
    }
}
unsafe extern "system" fn find(h: HMONITOR, _: HDC, _: *mut RECT, p: LPARAM) -> BOOL {
    let state = &mut *(p as *mut (String, HMONITOR));
    let mut info: MONITORINFOEXW = zeroed();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(h, &mut info as *mut _ as _) != 0 {
        let name = String::from_utf16_lossy(
            &info.szDevice[..info.szDevice.iter().position(|v| *v == 0).unwrap_or(32)],
        );
        if name == state.0 {
            state.1 = h;
            return 0;
        }
    }
    1
}
unsafe fn open(id: &str) -> Result<Handles, String> {
    let mut state: (String, HMONITOR) = (super::display_restore::source(id), null_mut());
    EnumDisplayMonitors(
        null_mut(),
        std::ptr::null(),
        Some(find),
        &mut state as *mut _ as isize,
    );
    if state.1.is_null() {
        return Err("Monitor is not attached to this computer".into());
    }
    let mut n = 0;
    if GetNumberOfPhysicalMonitorsFromHMONITOR(state.1, &mut n) == 0 || n == 0 || n > 16 {
        return Err("DDC/CI unavailable".into());
    }
    let mut all: Vec<Physical> = (0..n).map(|_| zeroed()).collect();
    if GetPhysicalMonitorsFromHMONITOR(state.1, n, all.as_mut_ptr()) == 0 {
        return Err("Cannot open monitor DDC/CI".into());
    }
    let handles = Handles(all);
    if n != 1 {
        return Err("Input switching is ambiguous for this display path".into());
    }
    Ok(handles)
}
pub fn parse(caps: &str) -> Vec<u32> {
    // Locate the actual 60(...) feature, never an unrelated numeric value.
    let lower = caps.to_ascii_lowercase();
    let Some(start)=lower.find("vcp(")else{return vec![];};
    let lower=&lower[start+4..];
    let mut out=vec![];
    for (i, _) in lower.match_indices("60") {
        let depth=lower[..i].bytes().fold(0i32,|n,c|n+if c==b'(' {1}else if c==b')' {-1}else{0});
        if depth!=0 {continue;}
        if i > 0 && lower.as_bytes()[i - 1].is_ascii_hexdigit() {
            continue;
        }
        let rest = lower[i + 2..].trim_start();
        if let Some(rest) = rest.strip_prefix('(') {
            if let Some(end) = rest.find(')') {
                for t in rest[..end].split_whitespace() {
                    if let Ok(v) = u32::from_str_radix(t, 16) {
                        if (1..=255).contains(&v) && !out.contains(&v) {
                            out.push(v);
                        }
                    }
                }
                break;
            }
        }
    }
    out
}
#[derive(Clone,serde::Deserialize)]
struct Port {code:u32,name:String}
fn default_readback()->bool{true}
#[derive(Clone,serde::Deserialize)]
struct Profile {inputs:Vec<Port>,#[serde(default="default_readback")]trust_readback:bool}
#[derive(Default)]
struct Profiles {root:std::path::PathBuf,entries:BTreeMap<String,Profile>}
thread_local!{static PROFILES:RefCell<Profiles>=RefCell::new(Profiles::default());}
pub fn configure(root:&std::path::Path){
    let entries=std::fs::read(root.join("config/monitor-inputs.json")).ok().filter(|b|b.len()<=262144).and_then(|b|serde_json::from_slice::<BTreeMap<String,Profile>>(&b).ok()).filter(|m|m.len()<=64).unwrap_or_default();
    PROFILES.with(|s|*s.borrow_mut()=Profiles{root:root.into(),entries});
}
fn profile(id:&str)->Option<Profile>{PROFILES.with(|s|{let s=s.borrow();let recovery:serde_json::Value=serde_json::from_slice(&std::fs::read(s.root.join("config/display-recovery.json")).ok()?).ok()?;
    let path=recovery.get(id)?.get("monitor_path")?.as_str()?;
    let p=s.entries.get(path)?;
    let mut p=p.clone();let mut seen=std::collections::BTreeSet::new();
    p.inputs.retain(|i|(1..=255).contains(&i.code)&&!i.name.trim().is_empty()&&i.name.len()<=120&&seen.insert(i.code));p.inputs.truncate(16);Some(p)
})}
fn port_name(code:u32)->String{match code {1=>"VGA 1".into(),2=>"VGA 2".into(),3=>"DVI 1".into(),4=>"DVI 2".into(),15=>"DisplayPort 1".into(),16=>"DisplayPort 2".into(),17=>"HDMI 1".into(),18=>"HDMI 2".into(),_=>format!("Input 0x{code:02X}")}}
fn resolve_ports(caps:&str,override_profile:Option<&Profile>)->Vec<Port>{
    if let Some(p)=override_profile {if !p.inputs.is_empty(){return p.inputs.clone();}}
    parse(caps).into_iter().map(|code|Port{code,name:port_name(code)}).collect()
}
unsafe fn read(id: &str) -> Result<Vec<Device>, String> {
    let handles = open(id)?;
    let h = handles.0[0].handle;
    let (mut current, mut max) = (0, 0);
    let current=if GetVCPFeatureAndVCPFeatureReply(h,0x60,null_mut(),&mut current,&mut max)!=0 {Some(current&255)}else{None};
    let mut len = 0;
    let mut caps = String::new();
    if GetCapabilitiesStringLength(h, &mut len) != 0 && len > 0 && len < 65536 {
        let mut b = vec![0; len as usize];
        if CapabilitiesRequestAndCapabilitiesReply(h, b.as_mut_ptr(), len) != 0 {
            caps = String::from_utf8_lossy(&b).trim_end_matches('\0').into();
        }
    }
    let calibration=profile(id);
    let selected=if calibration.as_ref().is_some_and(|p|!p.trust_readback){LAST_SOURCE.with(|s|s.borrow().get(id).copied())}else{current};
    Ok(resolve_ports(&caps,calibration.as_ref()).into_iter().map(|p|Device{id:p.code.to_string(),name:p.name,selected:Some(p.code)==selected}).collect())
}

thread_local! {static LAST_SOURCE:RefCell<BTreeMap<String,u32>>=RefCell::new(BTreeMap::new());}
thread_local! {static CACHE:RefCell<BTreeMap<String,(Instant,Vec<Device>)>>=RefCell::new(BTreeMap::new());}
pub unsafe fn snapshot(displays: &[Display]) -> BTreeMap<String, Vec<Device>> {
    let mut result = BTreeMap::new();
    for d in displays.iter().filter(|d| d.enabled) {
        let cached = CACHE.with(|c| {
            c.borrow()
                .get(&d.id)
                .filter(|(t, _)| t.elapsed() < Duration::from_secs(30))
                .map(|(_, v)| v.clone())
        });
        let rows = cached.unwrap_or_else(|| {
            let rows = read(&d.id).unwrap_or_default();
            CACHE.with(|c| {
                c.borrow_mut()
                    .insert(d.id.clone(), (Instant::now(), rows.clone()));
            });
            rows
        });
        result.insert(d.id.clone(), rows);
    }
    result
}
pub unsafe fn set(id: &str, value: u32) -> Result<(), String> {
    if !read(id)?.iter().any(|d| d.id == value.to_string()) {
        return Err("Input is not advertised by this monitor; calibrate it first".into());
    }
    let h = open(id)?;
    if SetVCPFeature(h.0[0].handle, 0x60, value) == 0 {
        return Err("Monitor rejected input switching".into());
    }
    LAST_SOURCE.with(|s|{s.borrow_mut().insert(id.into(),value);});
    CACHE.with(|c| {
        c.borrow_mut().remove(id);
    });
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generic_monitors_use_their_advertised_ports_without_pc_labels(){
        let ports=resolve_ports("vcp(60(0F 11 12))",None);
        assert_eq!(ports.iter().map(|p|p.name.as_str()).collect::<Vec<_>>(),vec!["DisplayPort 1","HDMI 1","HDMI 2"]);
        assert!(resolve_ports("vcp(10 12)",None).is_empty());
        let calibrated=Profile{inputs:vec![Port{code:41,name:"Custom input".into()}],trust_readback:false};
        assert_eq!(resolve_ports("vcp(10)",Some(&calibrated))[0].code,41);
        assert_eq!(port_name(25),"Input 0x19");
    }
    #[test]
    fn parse_only_input_feature() {
        assert_eq!(parse("vcp(10 60( 19 0F 11) 62)"), vec![25, 15, 17]);
        assert!(parse("vcp(10 12 DF 62)").is_empty());
        assert!(parse("vcp(160(11))").is_empty());
        assert!(parse("model(60(11))vcp(10)").is_empty());
        assert!(parse("vcp(E2(60(11)))").is_empty());
    }
}
