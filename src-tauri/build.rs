/// The Worker this app is built for.
///
/// This is an address, not a credential: it is the public origin of the Steamware proxy, and the only host the
/// app sends anything to. Every key it fronts (DepotBox, Steam, GitHub) lives on the Worker, so knowing this
/// URL gets nobody anything.
///
/// It used to be unset unless the build passed `STEAMWARE_API_BASE`, which meant a released installer shipped
/// with no server at all and greeted its first user by asking them to deploy a Cloudflare Worker — a step that
/// belongs to whoever builds this, not to whoever installs it. Baking it in makes the first run start at the
/// question that actually matters (recommended settings, then the plugin), and the Turnstile check on the
/// Worker is what gates the work from there.
const PRODUCTION_API_BASE: &str = "https://steamware-api.sparxreaderbypass.workers.dev";

fn main() {
    println!("cargo:rerun-if-env-changed=STEAMWARE_API_BASE");
    // The environment still wins, so a fork or a local backend can point the same source at its own Worker
    // without touching this file.
    let base = std::env::var("STEAMWARE_API_BASE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| PRODUCTION_API_BASE.to_string());
    println!("cargo:rustc-env=STEAMWARE_API_BASE={base}");

    tauri_build::build()
}
