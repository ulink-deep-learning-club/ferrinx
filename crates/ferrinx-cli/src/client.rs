use crate::config::CliConfig;
use crate::error::{CliError, Result};
use reqwest::multipart;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tokio_util::io::ReaderStream;

#[derive(Debug, Clone, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiResponse<T> {
    pub data: Option<T>,
    pub error: Option<ApiError>,
}

pub struct HttpClient {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl HttpClient {
    pub fn new(config: &CliConfig) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout))
            .connect_timeout(Duration::from_secs(10));

        if !config.verify_ssl {
            builder = builder.danger_accept_invalid_certs(true);
        }

        let client = builder.build()?;

        Ok(Self {
            client,
            base_url: config.api_url.clone(),
            api_key: config.api_key.clone(),
        })
    }

    pub fn set_api_key(&mut self, api_key: String) {
        self.api_key = Some(api_key);
    }

    pub fn clear_api_key(&mut self) {
        self.api_key = None;
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);

        let mut request = self.client.get(&url);

        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }

        let body: ApiResponse<T> = response.json().await?;

        body.data.ok_or_else(|| CliError::ApiError {
            code: "NO_DATA".to_string(),
            message: "No data in response".to_string(),
        })
    }

    pub async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);

        let mut request = self.client.post(&url).json(body);

        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }

        let body: ApiResponse<T> = response.json().await?;

        body.data.ok_or_else(|| CliError::ApiError {
            code: "NO_DATA".to_string(),
            message: "No data in response".to_string(),
        })
    }

    pub async fn post_raw<T: DeserializeOwned>(&self, path: &str, body: serde_json::Value) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);

        let mut request = self.client.post(&url).json(&body);

        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }

        let body: ApiResponse<T> = response.json().await?;

        body.data.ok_or_else(|| CliError::ApiError {
            code: "NO_DATA".to_string(),
            message: "No data in response".to_string(),
        })
    }

    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);

        let mut request = self.client.delete(&url);

        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }

        let body: ApiResponse<T> = response.json().await?;

        body.data.ok_or_else(|| CliError::ApiError {
            code: "NO_DATA".to_string(),
            message: "No data in response".to_string(),
        })
    }

    pub async fn delete_void(&self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);

        let mut request = self.client.delete(&url);

        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }

        let body: ApiResponse<serde_json::Value> = response.json().await?;

        if let Some(error) = body.error {
            return Err(CliError::ApiError {
                code: error.code,
                message: error.message,
            });
        }

        Ok(())
    }

    pub async fn put<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);

        let mut request = self.client.put(&url).json(body);

        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }

        let body: ApiResponse<T> = response.json().await?;

        body.data.ok_or_else(|| CliError::ApiError {
            code: "NO_DATA".to_string(),
            message: "No data in response".to_string(),
        })
    }

    pub async fn upload<T: DeserializeOwned>(
        &self,
        path: &str,
        file_path: &str,
        form_data: HashMap<String, String>,
    ) -> Result<T> {
        self.upload_with_config(path, file_path, form_data, None).await
    }

    pub async fn upload_with_config<T: DeserializeOwned>(
        &self,
        path: &str,
        file_path: &str,
        form_data: HashMap<String, String>,
        config_path: Option<&str>,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);

        let file = tokio::fs::File::open(file_path).await?;
        let file_size = file.metadata().await?.len();

        let file_name = file_path.split('/').next_back().unwrap_or("file");

        let stream = ReaderStream::new(file);
        let part = multipart::Part::stream_with_length(
            reqwest::Body::wrap_stream(stream),
            file_size,
        )
        .file_name(file_name.to_string());

        let mut form = multipart::Form::new().part("file", part);

        for (key, value) in form_data {
            form = form.text(key, value);
        }

        if let Some(cfg_path) = config_path {
            let config_content = tokio::fs::read_to_string(cfg_path).await?;
            form = form.text("config", config_content);
        }

        let mut request = self.client.post(&url).multipart(form);

        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }

        let body: ApiResponse<T> = response.json().await?;

        body.data.ok_or_else(|| CliError::ApiError {
            code: "NO_DATA".to_string(),
            message: "No data in response".to_string(),
        })
    }

    pub async fn upload_image<T: DeserializeOwned>(
        &self,
        path: &str,
        image_path: &str,
        form_data: HashMap<String, String>,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);

        let file = tokio::fs::File::open(image_path).await?;
        let file_size = file.metadata().await?.len();

        let file_name = image_path.split('/').next_back().unwrap_or("image");

        let stream = ReaderStream::new(file);
        let part = multipart::Part::stream_with_length(
            reqwest::Body::wrap_stream(stream),
            file_size,
        )
        .file_name(file_name.to_string())
        .mime_str("image/*")
        .map_err(|e| CliError::HttpError {
            status: 0,
            message: format!("MIME error: {}", e),
        })?;

        let mut form = multipart::Form::new().part("image", part);

        for (key, value) in form_data {
            form = form.text(key, value);
        }

        let mut request = self.client.post(&url).multipart(form);

        if let Some(ref api_key) = self.api_key {
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error_response(response).await);
        }

        let body: ApiResponse<T> = response.json().await?;

        body.data.ok_or_else(|| CliError::ApiError {
            code: "NO_DATA".to_string(),
            message: "No data in response".to_string(),
        })
    }

    async fn handle_error_response(&self, response: reqwest::Response) -> CliError {
        let status = response.status();

        match response.json::<ApiResponse<()>>().await {
            Ok(body) => {
                if let Some(error) = body.error {
                    CliError::ApiError {
                        code: error.code,
                        message: error.message,
                    }
                } else {
                    CliError::HttpError {
                        status: status.as_u16(),
                        message: status.to_string(),
                    }
                }
            }
            Err(_) => CliError::HttpError {
                status: status.as_u16(),
                message: status.to_string(),
            },
        }
    }
}
