use ab_pages::{Page, escape};

const SCRIPTS: &[&str] = &["/assets/account.js"];

fn render(title: &str, body: &str, nav: &str, wide: bool) -> String {
    Page {
        title,
        body,
        scripts: SCRIPTS,
        nav,
        wide,
    }
    .render()
}

pub(crate) fn sign_in() -> String {
    render(
        "Sign in",
        "<h1>Sign in</h1><p class=\"lede\">Manage the tunnels for your Macs.</p>\
         <form id=\"sign-in\" class=\"section stack\">\
         <label class=\"field\">Email<input type=\"email\" name=\"email\" autocomplete=\"username webauthn\" required></label>\
         <label class=\"field\">Password<input type=\"password\" name=\"password\" autocomplete=\"current-password\" required></label>\
         <button class=\"primary full\">Sign in</button></form>\
         <div class=\"divider\">or</div>\
         <button id=\"passkey-sign-in\" class=\"full\" type=\"button\">Sign in with a passkey</button>\
         <p id=\"status\" class=\"status small\" role=\"status\"></p>\
         <p class=\"muted small section\">New here? Accounts are by invite. Your invite link creates yours.</p>",
        "",
        false,
    )
}

pub(crate) fn invite() -> String {
    render(
        "Create your account",
        "<h1>Create your account</h1><p class=\"lede\">It lets you link your Macs to this relay. It never gets access to your data: each Mac approves that itself.</p>\
         <form id=\"sign-up\" class=\"section stack\" hidden>\
         <label class=\"field\">Name<input type=\"text\" name=\"name\" autocomplete=\"name\" required></label>\
         <label class=\"field\">Email<input type=\"email\" name=\"email\" autocomplete=\"username\" required></label>\
         <label class=\"field\">Password<input type=\"password\" name=\"password\" autocomplete=\"new-password\" minlength=\"12\" required></label>\
         <p class=\"muted small\">At least 12 characters.</p>\
         <button class=\"primary full\">Create account</button></form>\
         <p id=\"status\" class=\"status\" role=\"status\">Checking your invite…</p>",
        "",
        false,
    )
}

fn account_nav(email: &str) -> String {
    format!(
        "<span class=\"muted\">{}</span><button id=\"sign-out\" class=\"link\" type=\"button\">Sign out</button>",
        escape(email)
    )
}

pub(crate) fn device(email: &str, code: &str) -> String {
    let body = format!(
        "<h1>Link this Mac</h1><p class=\"lede\">Enter the code your Mac shows. Linking gives it a tunnel on this relay, and nothing here gets access to your data.</p>\
         <form id=\"device-lookup\" class=\"section stack\">\
         <label class=\"field\">Code from your Mac<input class=\"code-entry\" type=\"text\" name=\"code\" value=\"{}\" autocomplete=\"off\" autocapitalize=\"characters\" required></label>\
         <button class=\"primary full\">Continue</button></form>\
         <section id=\"device-confirm\" class=\"section\" hidden><h2>Is this your Mac?</h2>\
         <p id=\"device-warning\" class=\"status error\"><strong>Only continue if you started this yourself, on a Mac you can see right now.</strong> If someone sent you this code or link, or asked you to enter it, cancel: they are trying to attach their own Mac to your account.</p>\
         <dl class=\"facts\"><dt>Name it gave</dt><dd id=\"device-name\"></dd><dt>Key</dt><dd id=\"device-fingerprint\"></dd><dt>Asked</dt><dd id=\"device-age\"></dd><dt>From address</dt><dd id=\"device-source\"></dd></dl>\
         <p id=\"device-network\" class=\"small\"></p>\
         <p class=\"muted small\">The Mac chooses its own name, so a name proves nothing. The key must match what your Mac printed.</p>\
         <label class=\"field\"><input id=\"device-mine\" type=\"checkbox\"> I started this on my own Mac just now</label>\
         <div class=\"actions end\"><a class=\"button\" href=\"/\">Cancel</a><button id=\"device-allow\" class=\"primary\" type=\"button\" disabled>Link this Mac</button></div></section>\
         <section id=\"device-done\" class=\"section\" hidden><h2>Linked</h2><p class=\"muted\">Your Mac is finishing setup on its own. Next, connect an app using the address it shows.</p>\
         <div class=\"actions\"><a class=\"button\" href=\"/\">See your Macs</a></div></section>\
         <p id=\"status\" class=\"status small\" role=\"status\"></p>",
        escape(code),
    );
    render("Link this Mac", &body, &account_nav(email), false)
}

pub(crate) fn dashboard(email: &str) -> String {
    render(
        "Your Macs",
        "<div class=\"item-head\"><h1>Your Macs</h1><a class=\"button\" href=\"/device\">Link a Mac</a></div>\
         <p class=\"lede\">This account manages tunnels only. Access to a Mac's data is approved on that Mac.</p>\
         <div id=\"devices\" class=\"section\"><p class=\"muted\">Loading…</p></div>\
         <section class=\"section\"><h2>Passkeys</h2><p class=\"muted small\">Sign in here without a password.</p>\
         <div class=\"actions\"><button id=\"add-passkey\" type=\"button\">Add a passkey</button></div></section>\
         <p id=\"status\" class=\"status small\" role=\"status\"></p>",
        &account_nav(email),
        true,
    )
}
