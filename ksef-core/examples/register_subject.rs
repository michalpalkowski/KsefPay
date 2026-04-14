use ksef_core::domain::environment::KSeFEnvironment;
use ksef_core::domain::nip::Nip;
use ksef_core::infra::ksef::TestDataClient;

#[tokio::main]
async fn main() {
    let nip_str = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: register_subject <NIP>");
        eprintln!("Example: cargo run -p ksef-core --example register_subject -- 4583009462");
        std::process::exit(1);
    });

    let nip = Nip::parse(&nip_str).unwrap_or_else(|e| {
        eprintln!("Invalid NIP '{nip_str}': {e}");
        std::process::exit(1);
    });

    let client = TestDataClient::new(KSeFEnvironment::Test);

    println!("Registering subject NIP {nip} on KSeF test sandbox...");

    let (subject, perms) = client.setup_test_subject(&nip).await.unwrap_or_else(|e| {
        eprintln!("Failed: {e}");
        std::process::exit(1);
    });

    println!("Subject: {subject:?}");
    println!("Permissions: {perms:?}");
    println!("Done. You can now use KSEF_NIP={nip} in .env");
}
