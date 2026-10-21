//! Dev helper: point a Smart PIM database at the Dovecot fixture and sync.
//! Usage: cargo run -p smart-pim --example seed_mail -- <db-path> <passphrase>
use pfcore::store::Store;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: seed_mail <db-path> <passphrase>");
        std::process::exit(2);
    }
    let mut store = Store::open(std::path::Path::new(&args[1]), &args[2]).unwrap();
    let conn = store.conn_mut();
    let acct = pfcore::mailstore::create_account(
        conn,
        "Fixture",
        "imap",
        Some("127.0.0.1"),
        Some(10143),
        None,
    )
    .unwrap();
    conn.execute(
        "UPDATE mail_account SET security='none', user='test@example.com', \
         settings=json_set(settings,'$.password','secret123', \
         '$.smtp_host','127.0.0.1','$.smtp_port',1025) WHERE id=?1",
        [acct],
    )
    .unwrap();
    let reports = smart_pim_lib::mailsync::sync_imap_account(conn, acct).unwrap();
    println!("synced: {reports:?}");
}
