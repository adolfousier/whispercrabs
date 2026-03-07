use crate::config::Config;

#[test]
fn config_models_dir_exists_after_load() {
    let config = Config::load();
    assert!(config.models_dir.exists());
}

#[test]
fn config_db_path_is_under_data_dir() {
    let config = Config::load();
    assert!(config.db_path.to_str().unwrap().contains("whispercrabs"));
    assert!(config.db_path.to_str().unwrap().ends_with("history.db"));
}

#[test]
fn config_models_dir_is_under_data_dir() {
    let config = Config::load();
    assert!(config.models_dir.to_str().unwrap().contains("whispercrabs"));
    assert!(config.models_dir.to_str().unwrap().ends_with("models"));
}

#[test]
fn config_api_base_url_is_valid_url() {
    let config = Config::load();
    assert!(config.api_base_url.starts_with("http"));
}

#[test]
fn config_api_model_is_not_empty() {
    let config = Config::load();
    assert!(!config.api_model.is_empty());
}
