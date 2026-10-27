# update-check

Asks a web address whether a newer release of a program exists. No UI, no TLS
stack: the page is fetched by running `curl`.

```rust
match update_check::check(&settings.update_url, env!("CARGO_PKG_VERSION"), "myapp/1.2.3") {
    Ok(update_check::Status::Available(release)) => {
        // release.version is "1.5.0"; release.page, if any, is an http(s) address to open.
    }
    Ok(update_check::Status::UpToDate { .. }) => {}
    Err(e) => eprintln!("Couldn't check for updates: {e}"),
}
```

`check` blocks for up to `TIMEOUT_SECS` (15), so call it from a worker thread
and only when the user asks. Nothing in the crate runs by itself.

## Keep the address in your settings

The address is a parameter on purpose. Store it in the program's config file,
written out with its default, so it can be edited if the default's host goes
away while the program is still installed:

```json
{ "update_url": "https://api.github.com/repos/<owner>/<repo>/releases/latest" }
```

## What the address may answer

JSON in either shape:

- GitHub's latest release: `tag_name` is the version, `html_url` the page.
- Your own file, on any server: `{"version": "1.5.0", "url": "https://example.org/download"}`
  (`url` is optional).

A page that is not an `http` or `https` address is dropped, so a program may open
`release.page` without looking at it. Versions are dotted numbers with an optional
`v` and an optional pre-release (`1.5.0-rc.1`, older than `1.5.0`).

`cargo test -p update-check -- --ignored` also asks GitHub, to check that the
whole round trip works.
