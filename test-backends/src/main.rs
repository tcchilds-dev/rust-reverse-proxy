#[tokio::main]
async fn main() {
    let handles = vec![
        tokio::spawn(test_backends::run("A-1", 3001)),
        tokio::spawn(test_backends::run("A-2", 3002)),
        tokio::spawn(test_backends::run("A-3", 3003)),
        tokio::spawn(test_backends::run("B-1", 3005)),
    ];

    for handle in handles {
        handle.await.unwrap();
    }
}
