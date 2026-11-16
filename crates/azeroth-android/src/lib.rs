#![cfg(target_os = "android")]

use benilla_app::BuildId;
use bevy::prelude::*;

#[bevy_main]
fn main() {
    let android_app = bevy::android::ANDROID_APP.get().expect("Android activity");
    let Some(files) = android_app.internal_data_path() else {
        return;
    };
    std::env::set_var("WOW_DATA", files.join("Data"));
    std::env::set_var("BENILLA_HOME", files.join("benilla-config"));
    std::env::set_var("WGPU_BACKEND", "vulkan");
    benilla_app::run_survivors(BuildId {
        version: env!("CARGO_PKG_VERSION"),
        describe: env!("BENILLA_GIT_DESCRIBE"),
        sha: env!("BENILLA_GIT_SHA"),
        short: env!("BENILLA_GIT_SHORT"),
        date: env!("BENILLA_GIT_DATE"),
        profile: env!("BENILLA_PROFILE"),
        ..Default::default()
    });
}
