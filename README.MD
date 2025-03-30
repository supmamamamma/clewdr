# Clewd<span style="color:#CE422B">R</span>

Total rewrite of [Clewd修改版](https://github.com/teralomaniac/clewd) in Rust.

## **WORK IN PROGRESS**

**<span style="color:red">开发中，使用后果自负</span>**

## Advantages

- **No Node.js**: ~~Although I'm sure all of you playing SillyTavern have it installed~~
- **No external binaries**: Clewd spawns a `superfetch` command for every request to mimic browser behavior. ClewdR uses native Rust `rquest` library.
- **Support macOS**: Clewd doesn't provide `superfetch` for macOS, so it doesn't work on macOS.
- **Robust Backend**: Clewd uses home-made backend on top of Node's `Server` Object. ClewdR is built on top of `Axum` and `Tokio`, which are battle-tested libraries for building web servers in Rust.
- **Support Thinking**: Support newest `thinking` feature in SillyTavern for 3.7 Sonnet.
- **Support Image Attachment**: Support image attachment, you can send images to the bot.

## How to start

1. Download binary of your platform from GitHub release page.
2. Run `clewdr` / `clewdr.exe`.
3. A default config file `config.toml` will be generated in the same directory.
4. Edit `config.toml` to set your own config.

## Advanced usage

- When `cookie_array` is not empty and `cookie_index` is not negative, `clewdr` will use the cookie at `cookie_array[cookie index]` as the cookie for the request. And automatically rotate the cookie when needed.
- Store cookies you want to add in a txt file, one cookie per line. Pass the file path as first argument to `clewdr` or `clewdr.exe`. ClewdR will read the file save the cookies in `cookie_array`. E.g. `clewdr.exe cookie.txt` or `clewdr cookie.txt`. In desktop mode, you can simply drag and drop the file to the `clewdr` or `clewdr.exe` icon. The file path will be passed as the first argument.
- ClewdR will automatically sanitize cookies, cleaning up non-standard chars. But you need to ensure there are no extra numbers, letters, `_`, `=` or `-` in the cookie.
