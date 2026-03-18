#[tokio::main]
async fn main() {
    test_backends::run("A-1", 3001).await;
    test_backends::run("A-2", 3002).await;
    test_backends::run("A-3", 3003).await;
    test_backends::run("B-1", 3005).await;
}
