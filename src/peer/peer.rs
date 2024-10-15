pub struct Peer {
    pub username: String,
    pub connection_type: String,
    pub host: String,
    pub port: i32,
    pub token: i32,
    pub privileged: i8,
    pub unknown: i8,
    pub obfuscated_port: i8,
}
impl Peer {
    pub fn new(
        username: String,
        connection_type: String,
        host: String,
        port: i32,
        token: i32,
        privileged: i8,
        unknown: i8,
        obfuscated_port: i8,
    ) -> Self {
        Self {
            username,
            connection_type,
            host,
            port,
            token,
            privileged,
            unknown,
            obfuscated_port,
        }
    }
    pub fn print(&self) {
        println!("username: {}", self.username);
        println!("connection_type: {}", self.connection_type);
        println!("host: {}", self.host);
        println!("port: {}", self.port);
        println!("token: {}", self.token);
        println!("privileged: {}", self.privileged);
        println!("unknown: {}", self.unknown);
        println!("obfuscated_port: {}", self.obfuscated_port);
    }
}
