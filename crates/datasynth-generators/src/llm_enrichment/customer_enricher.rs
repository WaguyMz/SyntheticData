//! LLM-powered customer name enrichment.
//!
//! Generates realistic customer (company) names using an LLM provider in a single
//! batch request to avoid repetition, with deterministic template-based fallbacks.

use std::sync::Arc;

use datasynth_core::error::SynthError;
use datasynth_core::llm::{LlmProvider, LlmRequest};

/// Enriches customer metadata using an LLM provider.
///
/// Generates realistic customer/company names in one request for the whole set.
pub struct CustomerLlmEnricher {
    provider: Arc<dyn LlmProvider>,
}

impl CustomerLlmEnricher {
    /// Create a new enricher with the given LLM provider.
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// Generate all customer names in a single LLM request.
    ///
    /// Asks the LLM for exactly `count` distinct customer/company names in one call,
    /// to avoid repetition and ensure variety. Response is parsed as one name per line.
    /// When `response_in_french` is true (e.g. French GAAP), names must be in French.
    pub fn enrich_all_customer_names(
        &self,
        industry: &str,
        country: &str,
        count: usize,
        seed: u64,
        response_in_french: bool,
    ) -> Result<Vec<String>, SynthError> {
        if count == 0 {
            return Ok(Vec::new());
        }

        let lang_instruction = if response_in_french {
            " Use French legal forms (SARL, SAS, SA, etc.) where appropriate. Return company names in French (France)."
        } else {
            ""
        };
        let prompt = format!(
            "Generate exactly {} distinct, realistic but purely fictional customer/company names (buyers, clients) \
             that a {} company in {} would sell to. Use variety (B2B, B2C, different sectors and sizes). \
             Names must sound plausible and credible but must NOT reference or resemble any real company, brand, or trademark.{}\
             Return exactly one company name per line, no numbering, bullets, or extra text. Only the {} names, one per line.",
            count, industry, country, lang_instruction, count
        );

        let system = if response_in_french {
            "You are a synthetic data generator. Invent plausible, realistic company names that are purely fictional. Do not use or allude to any real companies, brands, or trademarks. Return only the requested number of company names in French, one per line, with no explanation or extra text."
        } else {
            "You are a synthetic data generator. Invent plausible, realistic company names that are purely fictional. Do not use or allude to any real companies, brands, or trademarks. Return only the requested number of company names, one per line, with no explanation or extra text."
        };
        let request = LlmRequest::new(prompt)
            .with_system(system.to_string())
            .with_max_tokens(2048)
            .with_temperature(0.8)
            .with_seed(seed);

        match self.provider.complete(&request) {
            Ok(response) => {
                let names = Self::parse_names_one_per_line(&response.content, count, |i| {
                    Self::fallback_customer_name(industry, country, i)
                });
                Ok(names)
            }
            Err(_) => Ok((0..count)
                .map(|i| Self::fallback_customer_name(industry, country, i))
                .collect()),
        }
    }

    fn parse_names_one_per_line<F>(content: &str, max_count: usize, fallback: F) -> Vec<String>
    where
        F: Fn(usize) -> String,
    {
        let lines: Vec<String> = content
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .take(max_count)
            .collect();
        let mut out = lines;
        while out.len() < max_count {
            out.push(fallback(out.len()));
        }
        out.truncate(max_count);
        out
    }

    fn fallback_customer_name(industry: &str, country: &str, index: usize) -> String {
        let prefix = match industry.to_lowercase().as_str() {
            "manufacturing" => "Mfg",
            "retail" => "Retail",
            "financial_services" | "finance" => "Finance",
            "healthcare" => "Health",
            "technology" => "Tech",
            _ => "Corp",
        };
        let suffix = match country.to_uppercase().as_str() {
            "US" | "USA" => " Inc",
            "DE" | "GERMANY" => " GmbH",
            "GB" | "UK" => " Ltd",
            "JP" | "JAPAN" => " KK",
            _ => " Co",
        };
        format!("{} Customer {}{}", prefix, index + 1, suffix)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use datasynth_core::llm::MockLlmProvider;

    #[test]
    fn test_enrich_all_customer_names_returns_count() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = CustomerLlmEnricher::new(provider);
        let names = enricher
            .enrich_all_customer_names("retail", "US", 5, 100, false)
            .expect("should succeed");
        assert_eq!(names.len(), 5);
        for name in &names {
            assert!(!name.is_empty());
        }
    }

    #[test]
    fn test_enrich_all_customer_names_zero() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = CustomerLlmEnricher::new(provider);
        let names = enricher
            .enrich_all_customer_names("tech", "DE", 0, 100, false)
            .expect("should succeed");
        assert!(names.is_empty());
    }
}
