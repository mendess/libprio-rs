// SPDX-License-Identifier: MPL-2.0

use criterion::{criterion_group, criterion_main, Criterion};
#[cfg(feature = "experimental")]
use criterion::{BenchmarkId, Throughput};
#[cfg(feature = "experimental")]
use fixed_macro::fixed;
#[cfg(feature = "multithreaded")]
use prio::flp::gadgets::ParallelSumMultithreaded;
use prio::{
    benchmarked::*,
    codec::Encode,
    field::{random_vector, FftFriendlyFieldElement, Field128 as F, FieldElement},
    flp::{
        gadgets::{BlindPolyEval, Mul, ParallelSum},
        types::SumVec,
        Type,
    },
    vdaf::{
        prio3::{Prio3, Prio3InputShare},
        Client as Prio3Client,
    },
};
#[cfg(feature = "prio2")]
use prio::{
    client::Client as Prio2Client,
    encrypt::PublicKey,
    server::{generate_verification_message, ValidationMemory},
};
#[cfg(feature = "experimental")]
use prio::{
    field::{Field255, Field64},
    idpf::{self, IdpfInput, RingBufferCache},
    vdaf::prg::PrgAes128,
};
#[cfg(feature = "experimental")]
use rand::random;
#[cfg(feature = "experimental")]
use std::iter;

/// This benchmark compares the performance of recursive and iterative FFT.
pub fn fft(c: &mut Criterion) {
    let test_sizes = [16, 256, 1024, 4096];
    for size in test_sizes.iter() {
        let inp = random_vector(*size).unwrap();
        let mut outp = vec![F::zero(); *size];

        c.bench_function(&format!("iterative FFT, size={}", *size), |b| {
            b.iter(|| {
                benchmarked_iterative_fft(&mut outp, &inp);
            })
        });

        c.bench_function(&format!("recursive FFT, size={}", *size), |b| {
            b.iter(|| {
                benchmarked_recursive_fft(&mut outp, &inp);
            })
        });
    }
}

/// Speed test for generating a seed and deriving a pseudorandom sequence of field elements.
pub fn prng(c: &mut Criterion) {
    let test_sizes = [16, 256, 1024, 4096];
    for size in test_sizes.iter() {
        c.bench_function(&format!("rand, size={}", *size), |b| {
            b.iter(|| random_vector::<F>(*size))
        });
    }
}

/// The asymptotic cost of polynomial multiplication is `O(n log n)` using FFT and `O(n^2)` using
/// the naive method. This benchmark demonstrates that the latter has better concrete performance
/// for small polynomials. The result is used to pick the `FFT_THRESHOLD` constant in
/// `src/flp/gadgets.rs`.
pub fn poly_mul(c: &mut Criterion) {
    let test_sizes = [1_usize, 30, 60, 90, 120, 150];
    for size in test_sizes.iter() {
        let m = (*size + 1).next_power_of_two();
        let mut g: Mul<F> = Mul::new(*size);
        let mut outp = vec![F::zero(); 2 * m];
        let inp = vec![random_vector(m).unwrap(); 2];

        c.bench_function(&format!("poly mul FFT, size={}", *size), |b| {
            b.iter(|| {
                benchmarked_gadget_mul_call_poly_fft(&mut g, &mut outp, &inp).unwrap();
            })
        });

        c.bench_function(&format!("poly mul direct, size={}", *size), |b| {
            b.iter(|| {
                benchmarked_gadget_mul_call_poly_direct(&mut g, &mut outp, &inp).unwrap();
            })
        });
    }
}

/// Benchmark generation and verification of boolean vectors.
pub fn count_vec(c: &mut Criterion) {
    let test_sizes = [10, 100, 1_000];
    for size in test_sizes.iter() {
        let input = vec![F::zero(); *size];

        #[cfg(feature = "prio2")]
        {
            // Public keys used to instantiate the v2 client.
            const PUBKEY1: &str = "BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9RqZx3XMkdEHBHfNuCSMpOwofVQ=";
            const PUBKEY2: &str = "BNNOqoU54GPo+1gTPv+hCgA9U2ZCKd76yOMrWa1xTWgeb4LhFLMQIQoRwDVaW64g/WTdcxT4rDULoycUNFB60LE=";

            // Prio2
            let pk1 = PublicKey::from_base64(PUBKEY1).unwrap();
            let pk2 = PublicKey::from_base64(PUBKEY2).unwrap();
            let mut client: Prio2Client<F> =
                Prio2Client::new(input.len(), pk1.clone(), pk2.clone()).unwrap();

            println!(
                "prio2 proof size={}\n",
                benchmarked_v2_prove(&input, &mut client).len()
            );

            c.bench_function(&format!("prio2 prove, input size={}", *size), |b| {
                b.iter(|| {
                    benchmarked_v2_prove(&input, &mut client);
                })
            });

            let input_and_proof = benchmarked_v2_prove(&input, &mut client);
            let mut validator: ValidationMemory<F> = ValidationMemory::new(input.len());
            let eval_at = random_vector(1).unwrap()[0];

            c.bench_function(&format!("prio2 query, input size={}", *size), |b| {
                b.iter(|| {
                    generate_verification_message(
                        input.len(),
                        eval_at,
                        &input_and_proof,
                        true,
                        &mut validator,
                    )
                    .unwrap();
                })
            });
        }

        // Prio3
        let count_vec: SumVec<F, ParallelSum<F, BlindPolyEval<F>>> = SumVec::new(1, *size).unwrap();
        let joint_rand = random_vector(count_vec.joint_rand_len()).unwrap();
        let prove_rand = random_vector(count_vec.prove_rand_len()).unwrap();
        let proof = count_vec.prove(&input, &prove_rand, &joint_rand).unwrap();

        println!("prio3 countvec proof size={}\n", proof.len());

        c.bench_function(
            &format!("prio3 countvec prove, input size={}", *size),
            |b| {
                b.iter(|| {
                    let prove_rand = random_vector(count_vec.prove_rand_len()).unwrap();
                    count_vec.prove(&input, &prove_rand, &joint_rand).unwrap();
                })
            },
        );

        c.bench_function(
            &format!("prio3 countvec query, input size={}", *size),
            |b| {
                b.iter(|| {
                    let query_rand = random_vector(count_vec.query_rand_len()).unwrap();
                    count_vec
                        .query(&input, &proof, &query_rand, &joint_rand, 1)
                        .unwrap();
                })
            },
        );

        #[cfg(feature = "multithreaded")]
        {
            let count_vec: SumVec<F, ParallelSumMultithreaded<F, BlindPolyEval<F>>> =
                SumVec::new(1, *size).unwrap();

            c.bench_function(
                &format!("prio3 countvec multithreaded prove, input size={}", *size),
                |b| {
                    b.iter(|| {
                        let prove_rand = random_vector(count_vec.prove_rand_len()).unwrap();
                        count_vec.prove(&input, &prove_rand, &joint_rand).unwrap();
                    })
                },
            );

            c.bench_function(
                &format!("prio3 countvec multithreaded query, input size={}", *size),
                |b| {
                    b.iter(|| {
                        let query_rand = random_vector(count_vec.query_rand_len()).unwrap();
                        count_vec
                            .query(&input, &proof, &query_rand, &joint_rand, 1)
                            .unwrap();
                    })
                },
            );
        }
    }
}

/// Benchmark prio3 client performance.
pub fn prio3_client(c: &mut Criterion) {
    let num_shares = 2;

    let prio3 = Prio3::new_aes128_count(num_shares).unwrap();
    let measurement = 1;
    println!(
        "prio3 count share size = {}",
        prio3_input_share_size(&prio3.shard(&measurement).unwrap().1)
    );
    c.bench_function("prio3 count", |b| {
        b.iter(|| {
            prio3.shard(&1).unwrap();
        })
    });

    let buckets: Vec<u64> = (1..10).collect();
    let prio3 = Prio3::new_aes128_histogram(num_shares, &buckets).unwrap();
    let measurement = 17;
    println!(
        "prio3 histogram ({} buckets) share size = {}",
        buckets.len() + 1,
        prio3_input_share_size(&prio3.shard(&measurement).unwrap().1)
    );
    c.bench_function(
        &format!("prio3 histogram ({} buckets)", buckets.len() + 1),
        |b| {
            b.iter(|| {
                prio3.shard(&measurement).unwrap();
            })
        },
    );

    let bits = 32;
    let prio3 = Prio3::new_aes128_sum(num_shares, bits).unwrap();
    let measurement = 1337;
    println!(
        "prio3 sum ({} bits) share size = {}",
        bits,
        prio3_input_share_size(&prio3.shard(&measurement).unwrap().1)
    );
    c.bench_function(&format!("prio3 sum ({bits} bits)"), |b| {
        b.iter(|| {
            prio3.shard(&measurement).unwrap();
        })
    });

    let len = 1000;
    let prio3 = Prio3::new_aes128_sum_vec(num_shares, 1, len).unwrap();
    let measurement = vec![0; len];
    println!(
        "prio3 countvec ({} len) share size = {}",
        len,
        prio3_input_share_size(&prio3.shard(&measurement).unwrap().1)
    );
    c.bench_function(&format!("prio3 countvec ({len} len)"), |b| {
        b.iter(|| {
            prio3.shard(&measurement).unwrap();
        })
    });

    #[cfg(feature = "multithreaded")]
    {
        let prio3 = Prio3::new_aes128_sum_vec_multithreaded(num_shares, 1, len).unwrap();
        let measurement = vec![0; len];
        println!(
            "prio3 countvec multithreaded ({} len) share size = {}",
            len,
            prio3_input_share_size(&prio3.shard(&measurement).unwrap().1)
        );
        c.bench_function(&format!("prio3 parallel countvec ({len} len)"), |b| {
            b.iter(|| {
                prio3.shard(&measurement).unwrap();
            })
        });
    }

    #[cfg(feature = "experimental")]
    {
        let len = 1000;
        let prio3 = Prio3::new_aes128_fixedpoint_boundedl2_vec_sum(num_shares, len).unwrap();
        let fp_num = fixed!(0.0001: I1F15);
        let measurement = vec![fp_num; len];
        println!(
            "prio3 fixedpoint16 boundedl2 vec ({} entries) size = {}",
            len,
            prio3_input_share_size(&prio3.shard(&measurement).unwrap().1)
        );
        c.bench_function(
            &format!("prio3 fixedpoint16 boundedl2 vec ({len} entries)"),
            |b| {
                b.iter(|| {
                    prio3.shard(&measurement).unwrap();
                })
            },
        );
    }

    #[cfg(all(feature = "experimental", feature = "multithreaded"))]
    {
        let prio3 =
            Prio3::new_aes128_fixedpoint_boundedl2_vec_sum_multithreaded(num_shares, len).unwrap();
        let fp_num = fixed!(0.0001: I1F15);
        let measurement = vec![fp_num; len];
        println!(
            "prio3 fixedpoint16 boundedl2 vec multithreaded ({} entries) size = {}",
            len,
            prio3_input_share_size(&prio3.shard(&measurement).unwrap().1)
        );
        c.bench_function(
            &format!("prio3 fixedpoint16 boundedl2 vec multithreaded ({len} entries)"),
            |b| {
                b.iter(|| {
                    prio3.shard(&measurement).unwrap();
                })
            },
        );
    }
}

fn prio3_input_share_size<F: FftFriendlyFieldElement, const L: usize>(
    input_shares: &[Prio3InputShare<F, L>],
) -> usize {
    let mut size = 0;
    for input_share in input_shares {
        size += input_share.get_encoded().len();
    }

    size
}

/// Benchmark IdpfPoplar performance.
#[cfg(feature = "experimental")]
pub fn idpf(c: &mut Criterion) {
    let test_sizes = [8usize, 8 * 16, 8 * 256];

    let mut group = c.benchmark_group("idpf_gen");
    for size in test_sizes.iter() {
        group.throughput(Throughput::Bytes(*size as u64 / 8));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let bits = iter::repeat_with(random).take(size).collect::<Vec<bool>>();
            let input = IdpfInput::from_bools(&bits);

            let mut inner_values = vec![[Field64::one(), Field64::zero()]; size - 1];
            for (value, random_element) in inner_values
                .iter_mut()
                .zip(random_vector::<Field64>(size - 1).unwrap())
            {
                value[1] = random_element;
            }
            let leaf_value = [Field255::one(), random_vector(1).unwrap()[0]];

            b.iter(|| {
                idpf::gen::<_, PrgAes128, 16, 2>(&input, inner_values.clone(), leaf_value).unwrap();
            });
        });
    }
    drop(group);

    let mut group = c.benchmark_group("idpf_eval");
    for size in test_sizes.iter() {
        group.throughput(Throughput::Bytes(*size as u64 / 8));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let bits = iter::repeat_with(random).take(size).collect::<Vec<bool>>();
            let input = IdpfInput::from_bools(&bits);

            let mut inner_values = vec![[Field64::one(), Field64::zero()]; size - 1];
            for (value, random_element) in inner_values
                .iter_mut()
                .zip(random_vector::<Field64>(size - 1).unwrap())
            {
                value[1] = random_element;
            }
            let leaf_value = [Field255::one(), random_vector(1).unwrap()[0]];

            let (public_share, keys) =
                idpf::gen::<_, PrgAes128, 16, 2>(&input, inner_values.clone(), leaf_value).unwrap();

            b.iter(|| {
                // This is an aggressively small cache, to minimize its impact on the benchmark.
                // In this synthetic benchmark, we are only checking one candidate prefix per level
                // instead of the usual two, so the cache hit rate will be unaffected.
                let mut cache = RingBufferCache::new(1);

                for prefix_length in 1..=size {
                    let prefix = input[..prefix_length].to_owned().into();
                    idpf::eval::<PrgAes128, 16, 2>(0, &public_share, &keys[0], &prefix, &mut cache)
                        .unwrap();
                }
            });
        });
    }
}

#[cfg(all(feature = "prio2", feature = "experimental"))]
criterion_group!(benches, count_vec, prio3_client, poly_mul, prng, fft, idpf);
#[cfg(all(not(feature = "prio2"), feature = "experimental"))]
criterion_group!(benches, prio3_client, poly_mul, prng, fft, idpf);
#[cfg(all(feature = "prio2", not(feature = "experimental")))]
criterion_group!(benches, count_vec, prio3_client, poly_mul, prng, fft);
#[cfg(all(not(feature = "prio2"), not(feature = "experimental")))]
criterion_group!(benches, prio3_client, poly_mul, prng, fft);

criterion_main!(benches);
