// Exposes a small global `window.aula.fb` that Rust calls via wasm-bindgen.
// The global is installed synchronously so the generated Dioxus module can call
// it even when Firebase's remote modules are still loading.
(function () {
  const firebaseConfig = {
    apiKey: window.__AULALITE_FIREBASE_API_KEY__,
    authDomain: window.__AULALITE_FIREBASE_AUTH_DOMAIN__,
    projectId: window.__AULALITE_FIREBASE_PROJECT_ID__,
  };

  let authPromise;

  function loadAuth() {
    if (!authPromise) {
      authPromise = Promise.all([
        import("https://www.gstatic.com/firebasejs/10.13.0/firebase-app.js"),
        import("https://www.gstatic.com/firebasejs/10.13.0/firebase-auth.js"),
      ]).then(([appModule, authModule]) => {
        const app = appModule.getApps().length
          ? appModule.getApp()
          : appModule.initializeApp(firebaseConfig);
        const auth = authModule.getAuth(app);
        return { auth, authModule };
      });
    }

    return authPromise;
  }

  window.aula = window.aula || {};
  window.aula.fb = {
    async signIn(email, password) {
      const { auth, authModule } = await loadAuth();
      const credential = await authModule.signInWithEmailAndPassword(auth, email, password);
      return credential.user.getIdToken();
    },

    async signUp(email, password) {
      const { auth, authModule } = await loadAuth();
      const credential = await authModule.createUserWithEmailAndPassword(auth, email, password);
      await authModule.sendEmailVerification(credential.user, {
        url: `${window.location.origin}/signup`,
      });

      // Invitation acceptance uses the verified email as an authorization
      // boundary. Keep the signup promise pending while the user verifies in
      // another tab, then force-refresh the token before the SPA calls /v1/me.
      // This gives the signup screen one coherent success path instead of
      // provisioning an unverified identity or navigating optimistically.
      const deadline = Date.now() + 10 * 60 * 1000;
      while (Date.now() < deadline) {
        await new Promise((resolve) => window.setTimeout(resolve, 3000));
        await authModule.reload(credential.user);
        if (credential.user.emailVerified) {
          return credential.user.getIdToken(true);
        }
      }

      throw new Error(
        "Verification link sent. Verify your email, then return and sign in."
      );
    },

    async signOut() {
      const { auth, authModule } = await loadAuth();
      await authModule.signOut(auth);
    },

    async forgot(email) {
      const { auth, authModule } = await loadAuth();
      await authModule.sendPasswordResetEmail(auth, email);
    },

    async currentIdToken() {
      const { auth } = await loadAuth();
      if (!auth.currentUser) {
        return null;
      }
      return auth.currentUser.getIdToken();
    },

    onIdTokenChanged(callback) {
      loadAuth()
        .then(({ auth, authModule }) => {
          authModule.onIdTokenChanged(auth, async (user) => {
            const token = user ? await user.getIdToken() : null;
            callback(token);
          });
        })
        .catch((error) => {
          console.error("Firebase auth initialization failed", error);
          callback(null);
        });
    },
  };

  // Deliberately do not preload Firebase here. Auth bootstrap, login or signup
  // is the demand signal; local-token/SSO sessions avoid two remote SDK loads.
})();
