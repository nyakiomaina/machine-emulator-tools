// Copyright Cartesi and individual authors (see AUTHORS)
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

use std::sync::Arc;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::env;

use actix_web::{web, middleware::Logger, web::Data, App, HttpResponse, HttpServer, http::header::CONTENT_TYPE};
use actix_web_validator::Json;
use async_mutex::Mutex;
use memmap2::MmapMut;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Notify;

use crate::config::Config;
use crate::rollup::{self, GIORequest, RollupFd};
use crate::rollup::{
    AdvanceRequest, Exception, FinishRequest, InspectRequest, Notice, Report, RollupRequest,
    Voucher,
};

use crate::get_blkgetsize64;

fn get_block_device_size(file: &File) -> Result<u64, HttpResponse> {
    get_blkgetsize64(file).map_err(|_| {
        HttpResponse::InternalServerError().body("Failed to get device size")
    })
}

fn init_state_drive() -> String {
    match env::var("STATE_DRIVE") {
        Ok(value) => value,
        Err(_) => {
            let default_value = "/dev/pmem1".to_string();
            env::set_var("STATE_DRIVE", &default_value);
            default_value
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "request_type")]
enum RollupHttpRequest {
    #[serde(rename = "advance_state")]
    Advance { data: AdvanceRequest },
    #[serde(rename = "inspect_state")]
    Inspect { data: InspectRequest },
}

/// Create new instance of http server
pub fn create_server(
    config: &Config,
    rollup_fd: Arc<Mutex<RollupFd>>,
) -> std::io::Result<actix_server::Server> {
    let state_drive = init_state_drive();

    let server = HttpServer::new(move || {
        let data = Data::new(Mutex::new(Context {
            rollup_fd: rollup_fd.clone(),
        }));
        App::new()
            .app_data(data)
            .app_data(Data::new(state_drive.clone()))
            .wrap(Logger::default())
            .service(voucher)
            .service(notice)
            .service(report)
            .service(gio)
            .service(exception)
            .service(finish)
            .service(raw_state_read)
            .service(raw_state_write)
            .service(raw_state_size)
    })
    .bind((config.http_address.as_str(), config.http_port))
    .map(|t| t)?
    .run();
    Ok(server)
}

/// Create and run new instance of http server
pub async fn run(
    config: &Config,
    rollup_fd: Arc<Mutex<RollupFd>>,
    server_ready: Arc<Notify>,
) -> std::io::Result<()> {
    log::info!("starting http dispatcher http service!");
    let server = create_server(config, rollup_fd)?;
    server_ready.notify_one();
    server.await
}

/// Process voucher request from DApp, write voucher to rollup device
#[actix_web::post("/voucher")]
async fn voucher(mut voucher: Json<Voucher>, data: Data<Mutex<Context>>) -> HttpResponse {
    log::debug!("received voucher request");
    // Check if address is valid
    if voucher.destination.len() != (rollup::CARTESI_ROLLUP_ADDRESS_SIZE * 2 + 2) as usize
        || (!voucher.destination.starts_with("0x"))
    {
        log::error!(
            "address not valid: '{}' len: {}",
            voucher.destination,
            voucher.destination.len()
        );
        return HttpResponse::BadRequest()
            .append_header((CONTENT_TYPE, "text/plain"))
            .body("address not valid");
    }
    let context = data.lock().await;
    // Write voucher to linux rollup device
    return match rollup::rollup_write_voucher(&*context.rollup_fd.lock().await, &mut voucher.0) {
        Ok(voucher_index) => {
            log::debug!("voucher successfully inserted {:#?}", voucher);
            HttpResponse::Created().json(IndexResponse {
                index: voucher_index,
            })
        }
        Err(e) => {
            log::error!(
                "unable to insert voucher, error details: '{}'",
                e.to_string()
            );
            HttpResponse::BadRequest()
                .append_header((CONTENT_TYPE, "text/plain"))
                .body(format!("unable to insert voucher, error details: '{}'", e))
        }
    };
}

/// Process notice request from DApp, write notice to rollup device
#[actix_web::post("/notice")]
async fn notice(mut notice: Json<Notice>, data: Data<Mutex<Context>>) -> HttpResponse {
    log::debug!("received notice request");
    let context = data.lock().await;
    // Write notice to linux rollup device
    return match rollup::rollup_write_notice(&*context.rollup_fd.lock().await, &mut notice.0) {
        Ok(notice_index) => {
            log::debug!("notice successfully inserted {:#?}", notice);
            HttpResponse::Created().json(IndexResponse {
                index: notice_index,
            })
        }
        Err(e) => {
            log::error!("unable to insert notice, error details: '{}'", e);
            HttpResponse::BadRequest()
                .append_header((CONTENT_TYPE, "text/plain"))
                .body(format!("unable to insert notice, error details: '{}'", e))
        }
    };
}

/// Process report request from DApp, write report to rollup device
#[actix_web::post("/report")]
async fn report(report: Json<Report>, data: Data<Mutex<Context>>) -> HttpResponse {
    log::debug!("received report request");
    let context = data.lock().await;
    // Write report to linux rollup device
    return match rollup::rollup_write_report(&*context.rollup_fd.lock().await, &report.0) {
        Ok(_) => {
            log::debug!("report successfully inserted {:#?}", report);
            HttpResponse::Accepted().body("")
        }
        Err(e) => {
            log::error!("unable to insert report, error details: '{}'", e);
            HttpResponse::BadRequest()
                .append_header((CONTENT_TYPE, "text/plain"))
                .body(format!("unable to insert notice, error details: '{}'", e))
        }
    };
}

/// Process gio request and return the result
#[actix_web::post("/gio")]
async fn gio(request: Json<GIORequest>, data: Data<Mutex<Context>>) -> HttpResponse {
    log::debug!("received gio request {:#?}", request);
    let context = data.lock().await;
    return match rollup::gio_request(&*context.rollup_fd.lock().await, &request.0) {
        Ok(result) => {
            log::debug!("gio successfully processed, response: {:#?}", result);
            HttpResponse::Accepted().body(json!(result).to_string())
        }
        Err(e) => {
            log::error!("unable to process gio request, error details: '{}'", e);
            HttpResponse::BadRequest()
                .append_header((CONTENT_TYPE, "text/plain"))
                .body(format!(
                    "unable to process gio request, error details: '{}'",
                    e
                ))
        }
    };
}

/// The DApp should call this method when it cannot proceed with the request processing after an exception happens.
/// This method should be the last method ever called by the DApp backend, and it should not expect the call to return.
/// The Rollup HTTP Server will pass the exception info to the Cartesi Server Manager.
#[actix_web::post("/exception")]
async fn exception(exception: Json<Exception>, data: Data<Mutex<Context>>) -> HttpResponse {
    log::debug!("received exception request {:#?}", exception);

    let context = data.lock().await;
    // Throw an exception
    return match rollup::rollup_throw_exception(&*context.rollup_fd.lock().await, &exception.0) {
        Ok(_) => {
            log::debug!("exception successfully thrown {:#?}", exception);
            HttpResponse::Accepted().body("")
        }
        Err(e) => {
            log::error!("unable to throw exception, error details: '{}'", e);
            HttpResponse::BadRequest()
                .append_header((CONTENT_TYPE, "text/plain"))
                .body(format!("unable to throw exception, error details: '{}'", e))
        }
    };
}

/// Process finish request from DApp, write finish to rollup device
/// and pass RollupFinish struct to linux rollup advance/inspect requests loop thread
#[actix_web::post("/finish")]
async fn finish(finish: Json<FinishRequest>, data: Data<Mutex<Context>>) -> HttpResponse {
    log::debug!("received finish request {:#?}", finish);
    // Prepare finish status for the rollup manager
    let accept = match finish.status.as_str() {
        "accept" => true,
        "reject" => false,
        _ => {
            return HttpResponse::BadRequest()
                .append_header((CONTENT_TYPE, "text/plain"))
                .body("status must be 'accept' or 'reject'");
        }
    };
    log::debug!(
        "request finished, writing to driver result `{}` ...",
        accept
    );
    let context = data.lock().await;
    let rollup_fd = context.rollup_fd.lock().await;
    // Write finish request, read indicator for next request
    let new_rollup_request = match rollup::perform_rollup_finish_request(&*rollup_fd, accept).await
    {
        Ok(finish_request) => {
            // Received new request, process it
            log::info!(
                "received new request of type {}",
                match finish_request.next_request_type {
                    0 => "ADVANCE",
                    1 => "INSPECT",
                    _ => "UNKNOWN",
                }
            );
            match rollup::handle_rollup_requests(&*rollup_fd, finish_request).await {
                Ok(rollup_request) => rollup_request,
                Err(e) => {
                    let error_message = format!(
                        "error performing handle_rollup_requests: `{}`",
                        e.to_string()
                    );
                    log::error!("{}", &error_message);
                    return HttpResponse::BadRequest()
                        .append_header((CONTENT_TYPE, "text/plain"))
                        .body(error_message);
                }
            }
        }
        Err(e) => {
            let error_message = format!(
                "error performing initial finish request: `{}`",
                e.to_string()
            );
            log::error!("{}", &error_message);
            return HttpResponse::BadRequest()
                .append_header((CONTENT_TYPE, "text/plain"))
                .body(error_message);
        }
    };

    // Respond to Dapp with the new rollup request
    let http_rollup_request = match new_rollup_request {
        RollupRequest::Advance(advance_request) => RollupHttpRequest::Advance {
            data: advance_request,
        },
        RollupRequest::Inspect(inspect_request) => RollupHttpRequest::Inspect {
            data: inspect_request,
        },
    };
    HttpResponse::Ok()
        .append_header((CONTENT_TYPE, "application/json"))
        .json(http_rollup_request)
}

// read from raw state
#[actix_web::get("/raw_state_read/{offset}/{size}")]
async fn raw_state_read(
    request_path: web::Path<(usize, usize)>, // Renamed `path` to avoid conflict.
    state_drive: web::Data<String>,
) -> HttpResponse {
    let (offset, size) = request_path.into_inner();
    let file = match File::open(&**state_drive) {
        Ok(f) => f,
        Err(_) => return HttpResponse::InternalServerError().body("Failed to open pmem device"),
    };

    let block_device_size = match get_block_device_size(&file) {
        Ok(size) => size,
        Err(resp) => return resp,
    };

    let mmap = unsafe { MmapMut::map_mut(&file).expect("Failed to map the file") }; // Use MmapMut.

    if offset + size > block_device_size as usize {
        return HttpResponse::BadRequest().body("Offset and size exceed memory bounds");
    }

    let data = &mmap[offset..offset + size];
    HttpResponse::Ok()
        .content_type("application/octet-stream")
        .body(data.to_vec())
}

// write to raw state
#[actix_web::post("/raw_state_write/{offset}")]
async fn raw_state_write(
    request_path: web::Path<usize>,
    body: web::Bytes,
    state_drive: web::Data<String>,
) -> HttpResponse {
    let offset = request_path.into_inner();
    let file = match OpenOptions::new().read(true).write(true).open(&**state_drive) {
        Ok(f) => f,
        Err(_) => return HttpResponse::InternalServerError().body("Failed to open pmem device"),
    };

    let _block_device_size = match get_block_device_size(&file) {
        Ok(size) => size,
        Err(resp) => return resp,
    };

    let mut mmap = unsafe { MmapMut::map_mut(&file).expect("Failed to map the file") };

    if offset + body.len() > mmap.len() {
        return HttpResponse::BadRequest().body("Offset and size exceed memory bounds");
    }

    mmap[offset..offset + body.len()].copy_from_slice(&body);
    mmap.flush().expect("Failed to flush the changes");

    HttpResponse::Ok().body("Data written successfully")
}

// get raw state size
#[actix_web::get("/raw_state_size")]
async fn raw_state_size(
    state_drive: web::Data<String>,
) -> HttpResponse {
    let file = match File::open(&**state_drive) {
        Ok(f) => f,
        Err(_) => return HttpResponse::InternalServerError().body("Failed to open pmem device"),
    };

    let block_device_size = match get_block_device_size(&file) {
        Ok(size) => size,
        Err(resp) => return resp,
    };

    HttpResponse::Ok().json(json!({ "size": block_device_size }))
}

#[derive(Debug, Clone, Serialize)]
struct IndexResponse {
    index: u64,
}

#[derive(Debug, Clone, Serialize)]
struct ErrorDescription {
    code: u16,
    reason: String,
    description: String,
}

#[derive(Debug, Serialize)]
struct Error {
    error: ErrorDescription,
}

struct Context {
    pub rollup_fd: Arc<Mutex<RollupFd>>,
}
