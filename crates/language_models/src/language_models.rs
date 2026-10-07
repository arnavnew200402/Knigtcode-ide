use std::sync::Arc;

use client::{Client, UserStore};
use collections::HashSet;
use credentials_provider::CredentialsProvider;
use gpui::{App, Context, Entity};
use language_model::LanguageModelRegistry;

pub mod extension;
pub mod provider;
mod settings;

pub use crate::extension::init_proxy as init_extension_proxy;

pub use crate::provider::mistral::MistralLanguageModelProvider;
pub use crate::settings::*;

pub fn init(user_store: Entity<UserStore>, client: Arc<Client>, cx: &mut App) {
    let credentials_provider = client.credentials_provider();
    let registry = LanguageModelRegistry::global(cx);
    registry.update(cx, |registry, cx| {
        register_language_model_providers(
            registry,
            user_store,
            client.clone(),
            credentials_provider.clone(),
            cx,
        );
    });

    // Subscribe to extension store events to track LLM extension installations
    if let Some(extension_store) = extension_host::ExtensionStore::try_global(cx) {
        cx.subscribe(&extension_store, {
            let registry = registry.downgrade();
            move |extension_store, event, cx| {
                let Some(registry) = registry.upgrade() else {
                    return;
                };
                match event {
                    extension_host::Event::ExtensionInstalled(extension_id) => {
                        if let Some(manifest) = extension_store
                            .read(cx)
                            .extension_manifest_for_id(extension_id)
                        {
                            if !manifest.language_model_providers.is_empty() {
                                registry.update(cx, |registry, cx| {
                                    registry.extension_installed(extension_id.clone(), cx);
                                });
                            }
                        }
                    }
                    extension_host::Event::ExtensionUninstalled(extension_id) => {
                        registry.update(cx, |registry, cx| {
                            registry.extension_uninstalled(extension_id, cx);
                        });
                    }
                    extension_host::Event::ExtensionsUpdated => {
                        let mut new_ids = HashSet::default();
                        for (extension_id, entry) in extension_store.read(cx).installed_extensions()
                        {
                            if !entry.manifest.language_model_providers.is_empty() {
                                new_ids.insert(extension_id.clone());
                            }
                        }
                        registry.update(cx, |registry, cx| {
                            registry.sync_installed_llm_extensions(new_ids, cx);
                        });
                    }
                    _ => {}
                }
            }
        })
        .detach();

        // Initialize with currently installed extensions
        registry.update(cx, |registry, cx| {
            let mut initial_ids = HashSet::default();
            for (extension_id, entry) in extension_store.read(cx).installed_extensions() {
                if !entry.manifest.language_model_providers.is_empty() {
                    initial_ids.insert(extension_id.clone());
                }
            }
            registry.sync_installed_llm_extensions(initial_ids, cx);
        });
    }

    // Upstream also registers a provider for every `language_models.openai_compatible`
    // and `language_models.anthropic_compatible` entry, each holding an API key
    // in the OS keychain. KnightCode has exactly one provider and one login, and
    // every credential lives in the shared `auth.json`, so those registrations
    // are not made: a settings file that names one is inert rather than a second
    // way to spend an account the IDE knows nothing about.
}

fn register_language_model_providers(
    registry: &mut LanguageModelRegistry,
    _user_store: Entity<UserStore>,
    _client: Arc<Client>,
    _credentials_provider: Arc<dyn CredentialsProvider>,
    cx: &mut Context<LanguageModelRegistry>,
) {
    registry.register_provider(
        Arc::new(knightcode_models::KnightCodeLanguageModelProvider::new(cx)),
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::settings::SettingsStore;
    use anyhow::Result;
    use clock::FakeSystemClock;
    use feature_flags::FeatureFlagAppExt as _;
    use gpui::{AppContext as _, AsyncApp, BorrowAppContext as _};
    use http_client::FakeHttpClient;
    use language_model::IconOrSvg;
    use release_channel::AppVersion;
    use std::future::Future;
    use std::pin::Pin;

    struct FakeCredentialsProvider;

    impl CredentialsProvider for FakeCredentialsProvider {
        fn read_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
            Box::pin(async { Ok(None) })
        }

        fn write_credentials<'a>(
            &'a self,
            _url: &'a str,
            _username: &'a str,
            _password: &'a [u8],
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
            Box::pin(async { Ok(()) })
        }

        fn delete_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn init_test(cx: &mut App) -> (Arc<Client>, Arc<dyn CredentialsProvider>) {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        cx.set_global(db::AppDatabase::test_new());
        let app_version = AppVersion::global(cx);
        release_channel::init_test(app_version, release_channel::ReleaseChannel::Dev, cx);
        gpui_tokio::init(cx);
        cx.update_flags(false, Vec::new());

        let client = Client::new(
            Arc::new(FakeSystemClock::new()),
            FakeHttpClient::with_404_response(),
            cx,
        );
        (client, Arc::new(FakeCredentialsProvider))
    }

    fn provider_icons(registry: &LanguageModelRegistry, id: &str) -> Vec<IconOrSvg> {
        registry
            .providers()
            .into_iter()
            .filter(|provider| provider.id().0.as_ref() == id)
            .map(|provider| provider.icon())
            .collect()
    }

    /// §13's expectation, as a test rather than a grep: whatever the settings
    /// file names, exactly one provider ends up registered and it is ours.
    #[gpui::test]
    fn test_only_knightcode_is_registered_whatever_the_settings_say(cx: &mut App) {
        let (client, credentials_provider) = init_test(cx);
        let registry = cx.new(|_| LanguageModelRegistry::default());

        let content = serde_json::json!({
            "language_models": {
                "openai_compatible": {
                    "acme": { "api_url": "https://example.com", "available_models": [] },
                    "other": { "api_url": "https://example.com", "available_models": [] },
                },
                "anthropic_compatible": {
                    "third": { "api_url": "https://example.com", "available_models": [] },
                },
            }
        })
        .to_string();
        cx.update_global::<SettingsStore, _>(|store, cx| {
            store
                .set_user_settings(&content, cx)
                .expect("failed to parse test settings");
        });

        knightcode_engine::Engine::starting_for_tests(cx);
        let user_store = cx.new(|cx| UserStore::new(client.clone(), cx));
        registry.update(cx, |registry, cx| {
            register_language_model_providers(
                registry,
                user_store,
                client,
                credentials_provider,
                cx,
            );
        });

        let ids = registry.read_with(cx, |registry, _| {
            registry
                .providers()
                .into_iter()
                .map(|provider| provider.id().0.to_string())
                .collect::<Vec<_>>()
        });
        assert_eq!(ids, vec!["knightcode".to_string()]);
        assert!(provider_icons(&registry.read(cx), "acme").is_empty());
    }
}
