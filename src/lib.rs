use {
    daumdic, daummap,
    failure::{Error, Fail},
    futures::prelude::*,
    howto,
    lazy_static::lazy_static,
    regex::Regex,
    serde_derive::{Deserialize, Serialize},
    std::str::FromStr,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    daummap_app_key: String,
}

#[derive(Debug, Fail, PartialEq, Eq)]
pub enum RequestError {
    #[fail(display = "cannot parse request {}", _0)]
    CannotParseRequest(String),
    #[fail(display = "address is not found for {}", _0)]
    AddressNotFound(String),
    #[fail(display = "{} is not a valid command for airkorea", _0)]
    InvalidAirkoreaCommand(String),
    #[fail(display = "answer is not found for {}", _0)]
    HowtoNotFound(String),
}

#[derive(Debug, Clone)]
pub enum Response {
    Dictionary(daumdic::Search),
    AirPollution(airkorea::AirStatus),
    HowTo(howto::Answer),
}

#[derive(Debug, Clone)]
pub enum Request {
    Dictionary(String),
    AirPollution(String, String),
    HowTo(String),
}

impl FromStr for Request {
    type Err = Error;

    fn from_str(message: &str) -> Result<Self, Self::Err> {
        lazy_static! {
            static ref REGEX_DIC: Regex = Regex::new(r"^[dD](?:ic)? (.+)$").unwrap();
            static ref REGEX_AIR: Regex =
                Regex::new(r"^(air|pm|pm10|pm25|o3|so2|no2|co|so2) (.+)$").unwrap();
            static ref REGEX_HOWTO: Regex = Regex::new(r"^[hH](?:owto)? (.+)$").unwrap();
        }

        REGEX_DIC
            .captures(message)
            .map(|c| c.get(1).unwrap().as_str().to_owned())
            .map(Request::Dictionary)
            .or_else(|| {
                REGEX_AIR
                    .captures(message)
                    .map(|c| {
                        (
                            c.get(1).unwrap().as_str().to_owned(),
                            c.get(2).unwrap().as_str().to_owned(),
                        )
                    })
                    .map(|(s1, s2)| Request::AirPollution(s1, s2))
            })
            .or_else(|| {
                REGEX_HOWTO
                    .captures(message)
                    .map(|c| c.get(1).unwrap().as_str().to_owned())
                    .map(Request::HowTo)
            })
            .ok_or_else(|| RequestError::CannotParseRequest(message.to_string()).into())
    }
}

impl Request {
    pub fn request(self, config: &Config) -> impl Future<Item = Response, Error = Error> {
        use futures::future::Either;

        match self {
            Request::Dictionary(query) => Either::A(search_dic(&query)),
            Request::AirPollution(command, query) => Either::B(Either::A(search_air(
                &command,
                &query,
                &config.daummap_app_key,
            ))),
            Request::HowTo(query) => Either::B(Either::B(search_howto(&query))),
        }
    }
}

fn join<T, U>(e: (Option<T>, Option<U>)) -> Option<(T, U)> {
    match e {
        (Some(t), Some(u)) => Some((t, u)),
        _ => None,
    }
}

fn get_coord_from_address(address: &daummap::Address) -> Option<(f32, f32)> {
    address
        .land_lot
        .as_ref()
        .map(|land_lot| (land_lot.longitude, land_lot.latitude))
        .and_then(join)
}

fn get_coord_from_place(place: &daummap::Place) -> Option<(f32, f32)> {
    join((place.longitude, place.latitude))
}

fn search_dic(query: &str) -> impl Future<Item = Response, Error = Error> {
    daumdic::search(query).map(Response::Dictionary)
}

fn search_air(
    command: &str,
    query: &str,
    app_key: &str,
) -> impl Future<Item = Response, Error = Error> {
    let command = command.to_string();
    let query = query.to_string();
    let app_key = app_key.to_string();

    daummap::AddressRequest::new(&app_key, &query)
        .get()
        .filter_map(|address| get_coord_from_address(&address))
        .into_future()
        .map_err(|(e, _)| e)
        .and_then({
            let query = query.clone();
            move |(o, _)| o.ok_or_else(|| RequestError::AddressNotFound(query).into())
        })
        .or_else({
            let query = query.clone();
            let app_key = app_key.clone();
            move |_| {
                daummap::KeywordRequest::new(&app_key, &query)
                    .get()
                    .filter_map(|place| get_coord_from_place(&place))
                    .into_future()
                    .map_err(|(e, _)| e)
                    .and_then(|(o, _)| o.ok_or_else(|| RequestError::AddressNotFound(query).into()))
            }
        })
        .and_then(|(longitude, latitude)| airkorea::search(longitude, latitude))
        .and_then(move |status| {
            let station_address = status.station_address.clone();
            let pollutants = match command.as_ref() {
                "air" => status.pollutants,
                "pm" => status
                    .into_iter()
                    .filter(|p| p.name.contains("PM"))
                    .collect(),
                command => status
                    .into_iter()
                    .filter(|p| p.name.to_lowercase().contains(&command))
                    .collect(),
            };

            if pollutants.is_empty() {
                Err(RequestError::InvalidAirkoreaCommand(command).into())
            } else {
                Ok(airkorea::AirStatus {
                    station_address,
                    pollutants,
                })
            }
        })
        .map(Response::AirPollution)
}

fn search_howto(query: &str) -> impl Future<Item = Response, Error = Error> {
    let query = query.to_string();
    howto::howto(&query)
        .into_future()
        .map_err(|(e, _)| e)
        .and_then(|(answer, _)| answer.ok_or_else(|| RequestError::HowtoNotFound(query).into()))
        .map(Response::HowTo)
}
