use codesubs_core::{Codesubs, ManagerConfig};

#[tokio::main]
async fn main() -> codesubs_core::Result<()> {
    let manager = Codesubs::new(ManagerConfig::from_environment()?)?;
    for account in manager.list().await? {
        let context = manager.env(&account.reference()).await?;
        println!("{} {}", account.reference(), context.home.display());
    }
    Ok(())
}
