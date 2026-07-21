//! 서비스 관리자 비밀번호의 bcrypt 해시를 생성한다.
//! 사용법: cargo run --example gen_admin_hash -- <평문 비밀번호>
fn main() {
    let password = std::env::args().nth(1).expect("usage: gen_admin_hash <password>");
    let hash = bcrypt::hash(&password, bcrypt::DEFAULT_COST).expect("hash 실패");
    println!("{hash}");
}
