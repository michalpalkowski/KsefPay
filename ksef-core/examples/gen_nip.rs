use rand::Rng;

fn main() {
    let weights: [u32; 9] = [6, 5, 7, 2, 3, 4, 5, 6, 7];
    let mut rng = rand::thread_rng();

    loop {
        let mut digits: Vec<u32> = (0..9)
            .map(|i| {
                if i == 0 {
                    rng.gen_range(1..=9)
                } else {
                    rng.gen_range(0..=9)
                }
            })
            .collect();

        let checksum: u32 = digits.iter().zip(&weights).map(|(d, w)| d * w).sum::<u32>() % 11;
        if checksum < 10 {
            digits.push(checksum);
            let nip: String = digits.iter().map(|d| char::from(b'0' + *d as u8)).collect();
            println!("{nip}");
            return;
        }
    }
}
