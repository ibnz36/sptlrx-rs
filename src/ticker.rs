use std::time::Duration;

use tokio::sync::mpsc::Sender;
use tokio::time;

use crate::AppEvent;

/// Emite `AppEvent::Tick` cada 50 ms (20 Hz).
/// El hilo de la UI usa cada tick para interpolar la posición actual
/// sin necesidad de consultar DBus en cada frame.
pub async fn run(tx: Sender<AppEvent>) {
    let mut interval = time::interval(Duration::from_millis(50));
    loop {
        interval.tick().await;
        if tx.send(AppEvent::Tick).await.is_err() {
            break; // El receptor (UI) fue cerrado
        }
    }
}
