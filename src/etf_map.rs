use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
struct TradingViewMapping {
    #[serde(rename = "TradingView_Master_Mapping")]
    sectors: Vec<Sector>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Sector {
    pub sector: String,
    pub sector_etf: String,
    pub industries: Vec<Industry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Industry {
    pub name: String,
    pub etf: String,
}

pub fn tv_mapping() -> Vec<Sector> {
    let json = include_str!("../sectors_industries_etf_map.json");
    let mapping = serde_json::from_str::<TradingViewMapping>(json).expect("Invalid JSON");
    mapping.sectors
}

#[cfg(test)]
mod test {
    use crate::etf_map::tv_mapping;

    #[test]
    fn print_mapping() {
        eprintln!("{:#?}", tv_mapping());
    }
}
