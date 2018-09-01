mod network;

mod remote_integration;
use remote_integration::RemoteIntegration;

mod jenkins_integration;
use jenkins_integration::*;

mod config_file;
use config_file::*;

mod jenkins_response;
use jenkins_response::*;

mod unity_cloud_response;
use unity_cloud_response::*;

mod team_city_response;
use team_city_response::*;

mod pin;
use pin::RgbLedLight;

mod errors;
use errors::UnityRetrievalError;

mod headers;

#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate failure;

#[macro_use]
extern crate log;
extern crate log4rs;

#[macro_use]
extern crate hyper;

extern crate chrono;
extern crate ctrlc;
extern crate reqwest;
extern crate serde;
extern crate serde_json;
extern crate toml;
extern crate wiringpi;

use std::fs::File;
use std::io::prelude::*;
use std::time::Duration;
use std::thread;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::panic;

use reqwest::{StatusCode, Url};
use reqwest::header::{qitem, Accept, Authorization, Basic, ContentType, Headers};
use reqwest::mime;
use failure::Error;
use chrono::prelude::*;

const SLEEP_DURATION: u64 = 10000;
const UNITY_SLEEP_DURATION: u64 = 1000 * 60;

lazy_static!{
    static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::new();
}

fn main() {
    let is_running_flag = Arc::new(AtomicBool::new(true));
    let r = is_running_flag.clone();
    ctrlc::set_handler(move || {
        info!("Ctrl-C received, signaling child threads to stop...");
        r.store(false, Ordering::SeqCst); // signal that main should stop.
    }).unwrap_or_else(|_| {
        error!("Error setting Ctrl-C handler.");
        panic!("Aborting...");
    });

    let failure_count = Arc::new(Mutex::new(0u32));
    match std::env::current_exe() {
        Ok(path) => {
            // Init logging
            let mut log_config_file_path = std::path::PathBuf::from(path.parent().unwrap());
            log_config_file_path.push("log4rs.yml");
            println!("Looking for log config file at: {:?}", log_config_file_path);
            log4rs::init_file(log_config_file_path, Default::default()).unwrap();

            // Init config file
            let mut config_file_path = std::path::PathBuf::from(path.parent().unwrap());
            config_file_path.push("config.toml");
            info!("Looking for config file at: {:?}", config_file_path);
            let mut config_file = File::open(config_file_path).unwrap_or_else(|err| {
                error!("No config.toml found in /src directory. Error: {}", err);
                panic!("Aborting...");
            });
            let mut config_text = String::new();
            config_file
                .read_to_string(&mut config_text)
                .unwrap_or_else(|err| {
                    error!("Failed to read config file. Error: {}", err);
                    panic!("Aborting...");
                });

            let config_values: Config =
                toml::from_str(config_text.as_str()).unwrap_or_else(|err| {
                    error!("Failed to deserialize config file. Error: {}", err);
                    panic!("Aborting...");
                });
            let jenkins_username = config_values.jenkins_username;
            let jenkins_password = config_values.jenkins_password;
            let jenkins_base_url = config_values.jenkins_base_url;
            let jenkins_running_flag = is_running_flag.clone();
            let (jenkins_r, jenkins_g, jenkins_b) = (
                config_values.jenkins_led_pins[0],
                config_values.jenkins_led_pins[1],
                config_values.jenkins_led_pins[2],
            );

            let unity_api_token = config_values.unity_cloud_api_token;
            let unity_base_url = config_values.unity_base_url;
            let unity_running_flag = is_running_flag.clone();
            let (unity_r, unity_g, unity_b) = (
                config_values.unity_led_pins[0],
                config_values.unity_led_pins[1],
                config_values.unity_led_pins[2],
            );

            let team_city_username = config_values.team_city_username;
            let team_city_password = config_values.team_city_password;
            let team_city_base_url = config_values.team_city_base_url;
            let team_city_running_flag = is_running_flag.clone();
            let (team_city_r, team_city_g, team_city_b) = (
                config_values.team_city_led_pins[0],
                config_values.team_city_led_pins[1],
                config_values.team_city_led_pins[2],
            );

            let allowed_total_failures = config_values.allowed_failures;
            // Init main threads
            let jenkins_counter = Arc::clone(&failure_count);
            let jenkins_handle = thread::spawn(move || {
               run_and_recover("Jenkins", allowed_total_failures, jenkins_counter, jenkins_running_flag.clone(), || {
                   start_jenkins_thread(
                        jenkins_r,
                        jenkins_g,
                        jenkins_b,
                        jenkins_username.as_str(),
                        jenkins_password.as_str(),
                        jenkins_base_url.as_str(),
                        jenkins_running_flag.clone())
               })
            });

            let unity_cloud_counter = Arc::clone(&failure_count);
            let unity_cloud_handle = thread::spawn(move || {
                run_and_recover("Unity Cloud", allowed_total_failures, unity_cloud_counter, unity_running_flag.clone(), || {
                     start_unity_thread(
                        unity_r,
                        unity_g,
                        unity_b,
                        unity_api_token.as_str(),
                        unity_base_url.as_str(),
                        unity_running_flag.clone())                                
                })
            });

            let team_city_counter = Arc::clone(&failure_count);
            let team_city_handle = thread::spawn(move || {                
                run_and_recover("Team City", allowed_total_failures, team_city_counter, team_city_running_flag.clone(), || {
                  start_team_city_thread(
                        team_city_r,
                        team_city_g,
                        team_city_b,
                        team_city_username.as_str(),
                        team_city_password.as_str(),
                        team_city_base_url.as_str(),
                        team_city_running_flag.clone())  
                })                
            });

            // Wait for all three main threads to finish.
            jenkins_handle.join().expect("The Jenkins thread terminated abnormally.");
            unity_cloud_handle.join().expect("The Unity Cloud build thread terminated abnormally.");
            team_city_handle.join().expect("The Team City thread terminated abnormally.");

            info!("All threads terminated. Terminating program...");
        }
        Err(e) => {
            error!("Failed to obtain current executable directory. Details: {}. Exiting...", e);
        }
    }
}

fn run_and_recover<F: Fn() -> R + panic::UnwindSafe + panic::RefUnwindSafe, R>(
    thread_name: &str, 
    allowed_total_failures: u32,
    failure_counter: Arc<Mutex<u32>>,
    running_flag: Arc<AtomicBool>,
    func: F
) -> thread::Result<R> 
where R: std::fmt::Debug {
    loop {
        if let Ok(counter) = failure_counter.lock() {
            if *counter > allowed_total_failures {
                running_flag.store(false, Ordering::SeqCst); // Force a global stop                
                return Result::Err(Box::new(format!("Failure count for {} exceeded, forcing stop.", thread_name)));
            }
        }
        let thread_result = panic::catch_unwind(|| {
            func()
        });
        if thread_result.is_ok() {
            info!("Thread {} terminated gracefully. Ending...", thread_name);
            return thread_result;
        } else {
            error!("Thread {} terminated abnormally. Details: {:?}. Restarting...", thread_name, thread_result);
            if let Ok(mut counter) = failure_counter.lock() {
                *counter += 1;
            }
            else {
                error!("Attempted to increment failure count for thread {}, but failed to acquire a lock on the counter.", thread_name);
            }
        }
    }
}

fn run_power_on_test(test_led: &mut pin::RgbLedLight) {
    test_led.turn_led_off();
    thread::sleep(Duration::from_millis(1000));
    test_led.set_led_rgb_values(RgbLedLight::RED);
    thread::sleep(Duration::from_millis(250));
    test_led.set_led_rgb_values(RgbLedLight::GREEN);
    thread::sleep(Duration::from_millis(250));
    test_led.set_led_rgb_values(RgbLedLight::BLUE);
    thread::sleep(Duration::from_millis(250));
    test_led.turn_led_off();
    thread::sleep(Duration::from_millis(250));
    test_led.set_led_rgb_values(RgbLedLight::WHITE);
    thread::sleep(Duration::from_millis(250));
    test_led.turn_led_off();

    test_led.glow_led(RgbLedLight::PURPLE);
}

fn start_thread<T: RemoteIntegration>(r: u16, g: u16, b: u16, remote: T, running_flag: Arc<AtomicBool>) {
    let mut led = RgbLedLight::new(r, g, b);
    run_power_on_test(&mut led);
    loop {
        remote.update_led(&mut led);
    }
    if !running_flag.load(Ordering::SeqCst) {
        led.glow_led(RgbLedLight::WHITE);
        thread::sleep(Duration::from_millis(1400)); // Should be long enough for a single "glow on -> glow off" cycle
        led.turn_led_off();
        return;
    }
}

fn start_jenkins_thread(
    r: u16,
    g: u16,
    b: u16,
    jenkins_username: &str,
    jenkins_password: &str,
    jenkins_base_url: &str,
    running_flag: Arc<AtomicBool>,
) {
    let mut jenkins_led = RgbLedLight::new(r, g, b);
    run_power_on_test(&mut jenkins_led);
    loop {
        run_one_jenkins(
            &mut jenkins_led,
            jenkins_username,
            jenkins_password,
            jenkins_base_url,
        );
        if !running_flag.load(Ordering::SeqCst) {
            jenkins_led.glow_led(RgbLedLight::WHITE);
            thread::sleep(Duration::from_millis(1400)); // Should be long enough for a single "glow on -> glow off" cycle
            jenkins_led.turn_led_off();
            return;
        }
    }
}

fn run_one_jenkins(
    jenkins_led: &mut RgbLedLight,
    jenkins_username: &str,
    jenkins_password: &str,
    jenkins_base_url: &str,
) {
    match get_jenkins_status(jenkins_username, jenkins_password, jenkins_base_url) {
        Ok(results) => {
            let (retrieved, not_retrieved): (
                Vec<Result<JenkinsBuildStatus, Error>>,
                Vec<Result<JenkinsBuildStatus, Error>>,
            ) = results.into_iter().partition(|x| x.is_ok());

            let retrieved: Vec<JenkinsBuildStatus> =
                retrieved.into_iter().map(|x| x.unwrap()).collect();
            
            let retrieved_count = retrieved.len();
            let not_retrieved_count = not_retrieved.len();
            let build_failures = *(&retrieved
                .iter()
                .filter(|x| **x == JenkinsBuildStatus::Failure || **x == JenkinsBuildStatus::Unstable)
                .count());
            let indeterminate_count = *(&retrieved
                .iter()
                .filter(|x| **x != JenkinsBuildStatus::Failure 
                            && **x != JenkinsBuildStatus::Unstable 
                            && **x != JenkinsBuildStatus::Success)
                .count()) + not_retrieved_count;
            let build_successes = *(&retrieved
                .iter()
                .filter(|x| **x == JenkinsBuildStatus::Success)
                .count());

            // Failure states: NONE of the builds succeeded.
            if build_successes <= 0 {
                if indeterminate_count > build_failures || build_failures == 0 {
                    // Glow blue if the majority of statuses are indeterminate, or if we have no success AND no failures
                    jenkins_led.glow_led(RgbLedLight::BLUE);
                } else {
                    jenkins_led.blink_led(RgbLedLight::RED);
                }
            }
            // Success, or partial success states: at least SOME builds succeeded.
            else {
                if build_failures == 0 {
                    // No failures, and more successes than indeterminates
                    if build_successes > indeterminate_count {
                        jenkins_led.set_led_rgb_values(RgbLedLight::GREEN);
                    }
                    // No failures, but more indeterminates that successes.
                    else {
                        jenkins_led.glow_led(RgbLedLight::TEAL);
                    }
                // Some failures, but more successes than failures
                } else if build_successes > build_failures {
                    jenkins_led.glow_led(RgbLedLight::YELLOW);
                // Many failures, more than successes.
                } else {
                    jenkins_led.blink_led(RgbLedLight::RED);
                }
            }

            info!("--Jenkins--: Retrieved {} jobs, failed to retrieve {} jobs. Of those, {} succeeded, {} failed, and {} were indeterminate.", retrieved_count, not_retrieved_count, build_successes, build_failures, indeterminate_count);
        }
        Err(e) => {
            jenkins_led.glow_led(RgbLedLight::BLUE);
            warn!(
                "--Jenkins--: Failed to retrieve any jobs from Jenkins. Details: {}",
                e
            );
        }
    }
    thread::sleep(Duration::from_millis(SLEEP_DURATION));
}

fn get_jenkins_status(
    username: &str,
    password: &str,
    base_url: &str,
) -> Result<Vec<Result<JenkinsBuildStatus, Error>>, Error> {
    let url_string = format!("{base}/api/json", base = base_url);
    let mut auth_headers = Headers::new();
    auth_headers.set(Authorization(get_basic_credentials(
        username,
        Some(password.to_string()),
    )));

    let all_jobs_response: Result<(JenkinsJobResponse, Headers), Error> =
        get_url_response(&url_string, auth_headers.clone());

    match all_jobs_response {
        Ok((result, _)) => {
            let results = result
                .jobs
                .iter()
                .filter(|job| job.color != JenkinsJobColor::Disabled
                                && job.color != JenkinsJobColor::DisabledAnime)
                .map(|job| {
                    let job_url_string = format!(
                        "{base}/job/{job}/lastBuild/api/json",
                        base = base_url,
                        job = job.name
                    );
                    let job_response: Result<
                        (JenkinsBuildResult, Headers),
                        Error,
                    > = get_url_response(&job_url_string, auth_headers.clone());

                    match job_response {                        
                        Ok((job_result, _)) => {
                            if job_result.building {                                
                                Ok(JenkinsBuildStatus::Building)
                            } else {
                                let unwrapped_result = job_result.build_result.unwrap();                                
                                Ok(unwrapped_result)
                            }
                        }
                        Err(job_err) => {
                            warn!("--Jenkins--: HTTP failure when attempting to get job result for job: {}. Error: {}", &job_url_string, job_err);
                            Err(job_err)
                        }
                    }
                })
                .collect();
            Ok(results)
        }
        Err(err) => Err(err),
    }
}

fn start_team_city_thread(
    r: u16,
    g: u16,
    b: u16,
    team_city_username: &str,
    team_city_password: &str,
    team_city_base_url: &str,
    running_flag: Arc<AtomicBool>,
) {
    let mut team_city_led = RgbLedLight::new(r, g, b);
    run_power_on_test(&mut team_city_led);
    loop {
        run_one_team_city(
            &mut team_city_led,
            team_city_username,
            team_city_password,
            team_city_base_url,
        );
        if !running_flag.load(Ordering::SeqCst) {
            team_city_led.glow_led(RgbLedLight::WHITE);
            thread::sleep(Duration::from_millis(1400)); // Should be long enough for a single "glow on -> glow off" cycle
            team_city_led.turn_led_off();
            return;
        }
    }
}

fn run_one_team_city(
    team_city_led: &mut RgbLedLight,
    team_city_username: &str,
    team_city_password: &str,
    team_city_base_url: &str,
) {
    let team_city_status =
        get_team_city_status(team_city_username, team_city_password, team_city_base_url);
    match team_city_status {
        Some(status) => match status {
            TeamCityBuildStatus::Success => team_city_led.set_led_rgb_values(RgbLedLight::GREEN),
            TeamCityBuildStatus::Failure => team_city_led.blink_led(RgbLedLight::RED),
            TeamCityBuildStatus::Error => team_city_led.glow_led(RgbLedLight::BLUE),
        },
        None => {
            team_city_led.glow_led(RgbLedLight::BLUE);
        }
    }

    thread::sleep(Duration::from_millis(SLEEP_DURATION));
}

fn get_team_city_status(
    username: &str,
    password: &str,
    base_url: &str,
) -> Option<TeamCityBuildStatus> {
    let url = format!("{base}/app/rest/builds/count:1", base = base_url);

    let mut headers = Headers::new();
    let auth_header = get_basic_credentials(username, Some(password.to_string()));
    // todo: check to see if we have a TCSESSION cookie, and use it instead of auth
    headers.set(Authorization(auth_header));
    headers.set(Accept(vec![qitem(mime::APPLICATION_JSON)]));

    let team_city_response: Result<(TeamCityResponse, Headers), Error> =
        get_url_response(url.as_str(), headers);
    match team_city_response {
        Ok((result, _)) => {
            // TODO: Get and return cookie for faster auth in the future
            info!("--Team City--: Build status: {:?}", result.status);
            Some(result.status)
        }
        Err(team_city_network_err) => {
            warn!(
                "--Team City--: Failed to get build status: {}",
                team_city_network_err
            );
            None
        }
    }
}

fn start_unity_thread(
    r: u16,
    g: u16,
    b: u16,
    unity_api_token: &str,
    unity_base_url: &str,
    running_flag: Arc<AtomicBool>,
) {
    let mut unity_led = RgbLedLight::new(r, g, b);
    run_power_on_test(&mut unity_led);
    let mut sleep_duration = UNITY_SLEEP_DURATION;
    loop {
        sleep_duration = run_one_unity(
            &mut unity_led,
            unity_api_token,
            unity_base_url,
            sleep_duration,
        );
        if !running_flag.load(Ordering::SeqCst) {
            unity_led.glow_led(RgbLedLight::WHITE);
            thread::sleep(Duration::from_millis(1400)); // Should be long enough for a single "glow on -> glow off" cycle
            unity_led.turn_led_off();
            return;
        }
    }
}

fn run_one_unity(
    unity_led: &mut RgbLedLight,
    unity_api_token: &str,
    unity_base_url: &str,
    mut sleep_duration: u64,
) -> u64 {
    let unity_results = get_unity_cloud_status(unity_api_token, unity_base_url);
    let (retrieved, not_retrieved): (
        Vec<Result<(UnityBuildStatus, Headers), UnityRetrievalError>>,
        Vec<Result<(UnityBuildStatus, Headers), UnityRetrievalError>>,
    ) = unity_results.into_iter().partition(|x| x.is_ok());

    let retrieved_results: Vec<(UnityBuildStatus, Headers)> =
        retrieved.into_iter().map(|x| x.unwrap()).collect();
    let not_retrieved_results: Vec<UnityRetrievalError> =
        not_retrieved.into_iter().map(|x| x.unwrap_err()).collect();

    if not_retrieved_results.len() > 0 {
        info!("--Unity--: At least one result not retrieved.");
        unity_led.glow_led(RgbLedLight::BLUE);
    } else {
        let passing_builds = *(&retrieved_results
            .iter()
            .filter(|x| x.0 == UnityBuildStatus::Success)
            .count());
        let failing_builds = *(&retrieved_results
            .iter()
            .filter(|x| x.0 == UnityBuildStatus::Failure)
            .count());
        let other_status_builds = *(&retrieved_results
            .iter()
            .filter(|x| x.0 != UnityBuildStatus::Success && x.0 != UnityBuildStatus::Failure)
            .count());

        // More misc statuses than knowns
        if other_status_builds > passing_builds + failing_builds {
            info!("--Unity--: More otherstatuses than passing AND failing.");
            unity_led.glow_led(RgbLedLight::BLUE);
        }
        // All passing or misc
        else if passing_builds > 0 && failing_builds == 0 {
            info!("--Unity--: All passing or misc.");
            unity_led.set_led_rgb_values(RgbLedLight::GREEN);
        }
        // All failing or misc
        else if passing_builds == 0 && failing_builds > 0 {
            info!("--Unity--: All failing or misc.");
            unity_led.blink_led(RgbLedLight::RED);
        }
        // Both failing and passing
        else if passing_builds > 0 && failing_builds > 0 {
            info!("--Unity--: At least one failing AND passing.");
            unity_led.glow_led(RgbLedLight::TEAL);
        }
        // ?????
        else {
            info!("--Unity--: Unknown state.");
            unity_led.glow_led(RgbLedLight::PURPLE);
        }

        info!(
            "--Unity--: {} passing builds, {} failing builds, {} builds with misc statuses.",
            passing_builds, failing_builds, other_status_builds
        );
    }

    // Adjust our timeout based on current rate limiting (if possible)
    if retrieved_results.len() > 0 {
        // Grab any of the headers at random
        let response_headers = &retrieved_results[0].1;
        if let Some(limit_remaining) = response_headers.get::<headers::XRateLimitRemaining>() {
            let limit_remaining = limit_remaining.0;
            if let Some(reset_timestamp_utc) = response_headers.get::<headers::XRateLimitReset>() {
                let reset_timestamp_utc = reset_timestamp_utc.0 as f32 / 1000f32; // Convert from milliseconds to seconds
                let now_unix_seconds = Utc::now().timestamp() as u64;
                let max_requests_per_second = limit_remaining as f32 / ((reset_timestamp_utc - now_unix_seconds as f32) as f32).max(1f32);
                let seconds_per_request = (1f32 / max_requests_per_second).max(UNITY_SLEEP_DURATION as f32);
                sleep_duration = seconds_per_request as u64;
            }
        }
    }
    
    thread::sleep(Duration::from_millis(sleep_duration));
    sleep_duration
}

fn get_unity_cloud_status(api_token: &str, base_url: &str) -> Vec<Result<(UnityBuildStatus, Headers), UnityRetrievalError>> {
    let mut headers = Headers::new();
    let auth_header = get_basic_credentials(api_token, None);
    headers.set(Authorization(auth_header));
    headers.set(ContentType::json());

    let ios_url = format!(
        "{base}/buildtargets/ios-development/builds?per_page=1",
        base = base_url
    );
    let ios_build_response = get_unity_platform_status(&headers, ios_url.as_str());

    let android_url = format!(
        "{base}/buildtargets/android-development/builds?per_page=1",
        base = base_url
    );
    let android_build_response = get_unity_platform_status(&headers, android_url.as_str());
    vec![ios_build_response, android_build_response]
}

fn get_unity_platform_status(headers: &Headers, url: &str,) -> Result<(UnityBuildStatus, Headers), UnityRetrievalError> {
    let unity_build_response: Result<(Vec<UnityBuild>, Headers), Error> = get_url_response(&url, headers.clone());
    match unity_build_response {
        Ok((mut unity_http_result, response_headers)) => {
            if unity_http_result.len() != 0 {
                Ok((unity_http_result.remove(0).build_status, response_headers))
            } else {
                warn!(
                    "--Unity--: No builds retrieved from Unity Cloud for URL {}. Aborting...",
                    url
                );
                Err(UnityRetrievalError::NoBuildsReturned)
            }
        }
        Err(unity_http_err) => {
            warn!(
                "--Unity--: Failure getting Unity Cloud build status for url: {}. Error: {}",
                url, unity_http_err
            );
            Err(UnityRetrievalError::HttpError {
                http_error_message: unity_http_err.to_string(),
            })
        }
    }
}

fn get_basic_credentials(username: &str, password: Option<String>) -> Basic {
    Basic {
        username: username.to_string(),
        password: password,
    }
}

fn get_url_response<T>(url_string: &str, headers: Headers) -> Result<(T, Headers), Error>
    where T: serde::de::DeserializeOwned,
{
    if let Ok(url) = Url::parse(&url_string) {
        let mut response = HTTP_CLIENT.get(url).headers(headers).send()?;

        match response.status() {
            StatusCode::Ok => {
                let body_string = response.text()?;
                let deser = serde_json::from_str::<T>(body_string.as_str())?;
                //todo: Do we have to clone this?
                Ok((deser, response.headers().clone()))
            }
            other_code => Err(format_err!(
                "HTTP call to {} failed with code: {}",
                &url_string,
                other_code
            )),
        }
    } else {
        Err(format_err!("Unable to parse url: {}", url_string))
    }
}
