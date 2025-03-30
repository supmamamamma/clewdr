# Clewd<span style="color:#CE422B">R</span>

Total rewrite of [Clewd修改版](https://github.com/teralomaniac/clewd) in Rust.

## **WORK IN PROGRESS**

**<span style="color:red">开发中，使用后果自负</span>**

## Advantages

1. **No Node.js**: ~~Although I'm sure all of you playing SillyTavern have it installed~~
2. **No external binaries**: Clewd spawns a `superfetch` command for every request to mimic browser behavior. ClewdR uses native Rust `rquest` library.
3. **Support macOS**: Clewd doesn't provide `superfetch` for macOS, so it doesn't work on macOS.
4. **Robust Backend**: Clewd uses home-made backend on top of Node's `Server` Object. ClewdR is built on top of `Axum` and `Tokio`, which are battle-tested libraries for building web servers in Rust.
5. **Support Thinking**: Support newest `thinking` feature in SillyTavern for 3.7 Sonnet.
6. **Support Image Attachment**: Support image attachment, you can send images to the bot.
