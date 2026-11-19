use subscription_core::{ManagerConfig, SubscriptionManager};

#[tokio::main]
async fn main() -> subscription_core::Result<()> {
    let manager = SubscriptionManager::new(ManagerConfig::from_environment()?)?;
    for account in manager.list().await? {
        let context = manager.env(&account.reference()).await?;
        println!("{} {}", account.reference(), context.home.display());
    }
    Ok(())
}
