mod authorization {
    mod tests {
        include!("contracts/authorization_tests.rs");
    }
}
mod delivery {
    mod tests {
        include!("contracts/delivery_tests.rs");
    }
}
mod runtime_client {
    mod tests {
        include!("contracts/runtime_client/tests.rs");
    }
}
mod semantic_sync {
    mod tests {
        include!("contracts/semantic_sync_tests.rs");
    }
}
mod viewport {
    mod bim {
        mod read_model {
            mod tests {
                include!("contracts/viewport/bim/read_model_tests.rs");
            }
        }
    }
}
