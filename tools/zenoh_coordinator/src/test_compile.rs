use zenoh::Wait;

async fn test_compile(session: &zenoh::Session) {
    let _sub = session.declare_subscriber("test").callback(|s| {}).await.unwrap();
}
