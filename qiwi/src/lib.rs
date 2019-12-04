//! Client for QIWI API based on [its official documentation](https://developer.qiwi.com/ru/qiwi-wallet-personal).
#![recursion_limit = "256"]

mod models;
mod transport;

pub use models::*;
pub use transport::*;

use async_stream::try_stream;
use bigdecimal::BigDecimal;
use chrono::prelude::*;
use http::Method;
use maplit::hashmap;
use penny::Currency;
use phonenumber::PhoneNumber;
use serde_json::json;
use std::{collections::HashMap, convert::TryFrom, fmt::Display, pin::Pin, sync::Arc};
use tokio_stream::*;

pub struct Client {
    caller: CallerWrapper,
    user: QiwiUser,
}

impl Client {
    pub fn new<T: Display>(phone: PhoneNumber, token: T) -> Self {
        let http_client = reqwest::Client::builder().build().unwrap();
        Self {
            caller: CallerWrapper {
                transport: Arc::new(RemoteCaller {
                    http_client,
                    addr: "https://edge.qiwi.com".into(),
                    bearer: Some(token.to_string()),
                }),
            },
            user: QiwiUser(phone),
        }
    }
}

impl Client {
    pub async fn profile_info(&self) -> anyhow::Result<ProfileInfo> {
        self
            .caller
            .call("person-profile/v1/profile/current", Method::GET, &hashmap! { "authInfoEnabled" => true.to_string(), "contractInfoEnabled" => true.to_string(), "userInfoEnabled" => true.to_string() }, None)
            .await?.into_result()
    }

    pub fn payment_history(
        &self,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<PaymentHistoryEntry>> + Send>> {
        let caller = self.caller.clone();
        let user_id = self.user.clone();
        Box::pin(try_stream! {
            let mut next_txn: Option<(String, u64)> = None;
            loop {
                let endpoint = format!("payment-history/v2/persons/{}/payments", user_id);
                let mut args = HashMap::new();
                args.insert("rows", 50.to_string());
                if let Some(next_txn) = next_txn.take() {
                    args.insert("nextTxnDate", next_txn.0.to_string());
                    args.insert("nextTxnId", next_txn.1.to_string());
                }
                let rsp = caller
                    .call(endpoint, Method::GET, &args, None)
                    .await?;

                let history: PaymentHistoryData = rsp.into_result()?;

                if let Some(date) = history.next_txn_date {
                    if let Some(id) = history.next_txn_id {
                        next_txn = Some((date, id));
                    }
                }

                for entry in history.data {
                    yield entry;
                }

                if next_txn.is_none() {
                    break;
                }
            }
        })
    }

    pub async fn commission_info(&self, provider: ProviderId) -> anyhow::Result<CommissionInfo> {
        let url = format!("sinap/providers/{}/form", provider);
        Ok(self
            .caller
            .call::<_, CommissionInfoWrapper>(url, Method::GET, &Default::default(), None)
            .await?
            .into_result()?
            .commission)
    }

    pub async fn commission_quote(
        &self,
        provider: ProviderId,
        account: PhoneNumber,
        amount: BigDecimal,
    ) -> anyhow::Result<BigDecimal> {
        let url = format!("sinap/providers/{}/onlineCommission", provider);
        let account = QiwiUser(account).to_string();
        Ok(self
            .caller
            .call::<_, CommissionQuote>(
                url,
                Method::POST,
                &Default::default(),
                Some(&json!({
                    "account": account,
                    "payment_method": {
                        "type": "Account",
                        "accountId": QiwiCurrency(Currency::RUB),
                    },
                    "purchaseTotals": {
                        "total": {
                            "amount": amount,
                            "currency": QiwiCurrency(Currency::RUB),
                        }
                    }
                })),
            )
            .await?
            .into_result()?
            .qw_commission
            .amount)
    }

    pub async fn transfer(
        &self,
        id: Option<u64>,
        amount: BigDecimal,
        direction: TransferDirection,
        comment: String,
    ) -> anyhow::Result<TransferData> {
        let (provider, sum_currency, account) = match direction {
            TransferDirection::Qiwi {
                to_phone,
                to_currency,
            } => (99, to_currency, to_phone),
            TransferDirection::Cellular { carrier, to_phone } => {
                (carrier, penny::Currency::RUB, to_phone)
            }
        };

        let url = format!("sinap/api/v2/terms/{}/payments", provider);

        self
            .caller
            .call(
                url,
                Method::POST,
                &Default::default(),
                Some(&json!({
                    "id": id.unwrap_or(u64::try_from(Utc::now().timestamp()).unwrap() * 1000).to_string(),
                    "sum": {
                        "amount": amount,
                        "currency": QiwiCurrency(sum_currency),
                    },
                    "paymentMethod": {
                        "type": "Account",
                        "accountId": QiwiCurrency(Currency::RUB),
                    },
                    "fields": {
                        "account": QiwiUser(account).to_string(),
                    },
                    "comment": comment,
                })),
            )
            .await?
            .into_result()
    }
}
