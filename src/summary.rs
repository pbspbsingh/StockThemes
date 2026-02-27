use crate::{Stock, Ticker};
use askama::Template;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub size: usize,
    pub sectors: Vec<SummarySector>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummarySector {
    pub name: String,
    pub url: String,
    pub size: usize,
    pub industries: Vec<SummaryIndustry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryIndustry {
    pub name: String,
    pub url: String,
    pub size: usize,
    pub tickers: Vec<Ticker>,
}

impl Summary {
    pub fn summarize(stocks: impl Iterator<Item = Stock>) -> Summary {
        let mut size = 0;
        let mut sectors = Vec::new();

        let sectors_map = stocks
            .into_iter()
            .into_group_map_by(|stock| stock.sector.name.clone());
        for (sector_name, stocks) in sectors_map {
            if stocks.is_empty() {
                continue;
            }

            let mut sector_summary = SummarySector {
                name: sector_name,
                url: stocks[0].sector.url.clone(),
                size: 0,
                industries: Vec::new(),
            };

            let industry_map = stocks
                .into_iter()
                .into_group_map_by(|stock| stock.industry.name.clone());
            for (industry_name, stocks) in industry_map {
                if stocks.is_empty() {
                    continue;
                }

                let industry_summary = SummaryIndustry {
                    name: industry_name,
                    url: stocks[0].industry.url.clone(),
                    size: stocks.len(),
                    tickers: stocks
                        .into_iter()
                        .map(|s| Ticker {
                            exchange: s.exchange,
                            ticker: s.ticker,
                        })
                        .collect(),
                };
                sector_summary.size += industry_summary.size;
                sector_summary.industries.push(industry_summary);
            }
            sector_summary
                .industries
                .sort_by_key(|si| -(si.size as isize));
            if sector_summary.size > 0 {
                size += sector_summary.size;
                sectors.push(sector_summary);
            }
        }
        sectors.sort_by_key(|ss| -(ss.size as isize));
        Summary { size, sectors }
    }

    pub fn render(
        &self,
        sector_rs: HashMap<String, f64>,
        industry_rs: HashMap<String, f64>,
        stock_rs: HashMap<String, f64>,
    ) -> String {
        #[derive(Template)]
        #[template(path = "./stocks_themes.html")]
        struct Html<'a> {
            summary: &'a Summary,
            sector_rs: HashMap<String, f64>,
            industry_rs: HashMap<String, f64>,
            stock_rs: HashMap<String, f64>,
        }

        let html = Html {
            summary: self,
            sector_rs,
            industry_rs,
            stock_rs,
        };

        html.render().expect("Failed to render ")
    }
}
