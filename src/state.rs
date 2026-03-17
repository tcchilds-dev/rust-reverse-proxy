use std::sync::Arc;

use anyhow::Result;

use crate::{
    balancer::{LoadBalancer, two_random_choices::TwoRandomChoicesBalancer},
    config::Config,
};

pub struct Route {
    pub path_prefix: String,
    pub balancer: Arc<dyn LoadBalancer>,
}

#[derive(Clone)]
pub struct AppState {
    pub client: reqwest::Client,
    pub routes: Arc<Vec<Route>>,
}

impl AppState {
    pub fn from_config(config: Config) -> Result<Self> {
        let client = reqwest::Client::new();

        let routes: Vec<Route> = config
            .routes
            .into_iter()
            .map(|r| Route {
                path_prefix: r.path_prefix,
                balancer: Arc::new(TwoRandomChoicesBalancer::new(r.backends)),
            })
            .collect();

        Ok(Self {
            client,
            routes: Arc::new(routes),
        })
    }
}
