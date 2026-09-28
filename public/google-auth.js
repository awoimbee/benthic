//! Google Identity Services helper for benthic's "Sign in with Google".
//
// Exposes two globals used by the web build (see src/sync.rs):
//
//   window.benthicGoogleLogin(clientId, scope) -> Promise<string>
//       Resolves with a JSON string `{"access_token": ..., "expires_in": ...}`
//       after the user signs in and grants the Drive scope.
//
//   window.benthicGoogleSignOut(accessToken) -> void
//       Best-effort token revocation.
//
// The OAuth client ID is a public identifier (not a secret). It is configured
// by the app owner, not the end user.
(function () {
  "use strict";

  var SOURCE = "https://accounts.google.com/gsi/client";

  function loaded() {
    return !!(window.google && window.google.accounts && window.google.accounts.oauth2);
  }

  function load() {
    if (loaded()) return Promise.resolve();
    if (window.__benthicGisLoading) return window.__benthicGisLoading;
    window.__benthicGisLoading = new Promise(function (resolve, reject) {
      var script = document.createElement("script");
      script.src = SOURCE;
      script.async = true;
      script.defer = true;
      script.onload = function () {
        resolve();
      };
      script.onerror = function () {
        reject(new Error("Could not load Google Identity Services. Check your connection."));
      };
      document.head.appendChild(script);
    });
    return window.__benthicGisLoading;
  }

  window.benthicGoogleLogin = function (clientId, scope) {
    if (!clientId) {
      return Promise.reject(new Error("No Google OAuth client ID is configured."));
    }
    return load().then(function () {
      return new Promise(function (resolve, reject) {
        var client = google.accounts.oauth2.initTokenClient({
          client_id: clientId,
          scope: scope,
          callback: function (response) {
            if (response && response.access_token) {
              resolve(
                JSON.stringify({
                  access_token: response.access_token,
                  expires_in: Number(response.expires_in) || 3600
                })
              );
            } else {
              reject(new Error((response && response.error) || "Sign-in was cancelled."));
            }
          },
          error_callback: function (error) {
            reject(new Error((error && error.type) || "Sign-in failed."));
          }
        });
        client.requestAccessToken();
      });
    });
  };

  window.benthicGoogleSignOut = function (accessToken) {
    try {
      if (loaded() && accessToken) google.accounts.oauth2.revoke(accessToken);
    } catch (error) {
      // Revocation is best-effort.
    }
  };
})();
