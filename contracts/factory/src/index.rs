use soroban_sdk::{Address, Env, Vec};

use crate::{query, storage::DataKey, ttl};

const PAGE_SIZE: u32 = query::MAX_PAGE_SIZE;

fn extend_ttl(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, ttl::THRESHOLD, ttl::EXTEND_TO);
}

fn write_page(env: &Env, key: DataKey, page: &Vec<u64>) {
    env.storage().persistent().set(&key, page);
    extend_ttl(env, &key);
}

fn migrate_legacy_index(
    env: &Env,
    legacy_key: &DataKey,
    count_key: &DataKey,
    mut make_page_key: impl FnMut(u32) -> DataKey,
) -> u32 {
    if let Some(count) = env.storage().persistent().get::<_, u32>(count_key) {
        return count;
    }

    let legacy: Option<Vec<u64>> = env.storage().persistent().get(legacy_key);
    let Some(entries) = legacy else {
        return 0;
    };

    let count = entries.len();
    let mut page = Vec::new(env);
    let mut page_index = 0_u32;

    for entry in entries.iter() {
        page.push_back(entry);
        if page.len() == PAGE_SIZE {
            write_page(env, make_page_key(page_index), &page);
            page = Vec::new(env);
            page_index = page_index.saturating_add(1);
        }
    }

    if page.len() > 0 {
        write_page(env, make_page_key(page_index), &page);
    }

    env.storage().persistent().set(count_key, &count);
    extend_ttl(env, count_key);
    env.storage().persistent().remove(legacy_key);

    count
}

fn append_index_entry(
    env: &Env,
    count_key: &DataKey,
    legacy_key: &DataKey,
    mut make_page_key: impl FnMut(u32) -> DataKey,
    entry: u64,
) {
    let count = migrate_legacy_index(env, legacy_key, count_key, &mut make_page_key);
    let page_index = count / PAGE_SIZE;
    let page_key = make_page_key(page_index);

    let mut page: Vec<u64> = env
        .storage()
        .persistent()
        .get(&page_key)
        .unwrap_or(Vec::new(env));
    page.push_back(entry);
    write_page(env, page_key, &page);

    let new_count = count.saturating_add(1);
    env.storage().persistent().set(count_key, &new_count);
    extend_ttl(env, count_key);
}

fn collect_page(
    env: &Env,
    make_page_key: &mut impl FnMut(u32) -> DataKey,
    page_index: u32,
) -> Vec<u64> {
    let key = make_page_key(page_index);
    if env.storage().persistent().has(&key) {
        extend_ttl(env, &key);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

fn read_index(
    env: &Env,
    count_key: &DataKey,
    legacy_key: &DataKey,
    mut make_page_key: impl FnMut(u32) -> DataKey,
    offset: u32,
    limit: u32,
) -> Vec<u64> {
    if let Some(count) = env.storage().persistent().get::<_, u32>(count_key) {
        extend_ttl(env, count_key);
        if offset >= count {
            return Vec::new(env);
        }

        let effective_limit = limit.min(PAGE_SIZE);
        let end = offset.saturating_add(effective_limit).min(count);
        let mut result = Vec::new(env);
        let mut cursor = offset;

        while cursor < end {
            let page_index = cursor / PAGE_SIZE;
            let page_offset = (cursor % PAGE_SIZE) as usize;
            let page = collect_page(env, &mut make_page_key, page_index);
            let page_len = page.len() as usize;

            if page_offset >= page_len {
                let next_page_start = page_index.saturating_add(1).saturating_mul(PAGE_SIZE);
                if next_page_start <= cursor {
                    break;
                }
                cursor = next_page_start;
                continue;
            }

            let remaining_in_page = page_len - page_offset;
            let remaining_total = (end - cursor) as usize;
            let take = remaining_in_page.min(remaining_total);
            for i in page_offset..(page_offset + take) {
                result.push_back(page.get(i as u32).unwrap());
            }
            cursor = cursor.saturating_add(take as u32);
        }

        return result;
    }

    let legacy: Vec<u64> = env
        .storage()
        .persistent()
        .get(legacy_key)
        .unwrap_or(Vec::new(env));
    if env.storage().persistent().has(legacy_key) {
        extend_ttl(env, legacy_key);
    }
    query::paginate(env, legacy, offset, limit)
}

fn count_index(env: &Env, count_key: &DataKey, legacy_key: &DataKey) -> u32 {
    if let Some(count) = env.storage().persistent().get::<_, u32>(count_key) {
        extend_ttl(env, count_key);
        return count;
    }

    let legacy: Vec<u64> = env
        .storage()
        .persistent()
        .get(legacy_key)
        .unwrap_or(Vec::new(env));
    if env.storage().persistent().has(legacy_key) {
        extend_ttl(env, legacy_key);
    }
    legacy.len()
}

pub fn append_sender_index(env: &Env, sender: &Address, stream_id: u64) {
    let count_key = DataKey::BySenderCount(sender.clone());
    let legacy_key = DataKey::BySender(sender.clone());
    append_index_entry(env, &count_key, &legacy_key, |page| {
        DataKey::BySenderPage(sender.clone(), page)
    }, stream_id);
}

pub fn append_recipient_index(env: &Env, recipient: &Address, stream_id: u64) {
    let count_key = DataKey::ByRecipientCount(recipient.clone());
    let legacy_key = DataKey::ByRecipient(recipient.clone());
    append_index_entry(env, &count_key, &legacy_key, |page| {
        DataKey::ByRecipientPage(recipient.clone(), page)
    }, stream_id);
}

pub fn streams_by_sender(env: &Env, sender: Address, offset: u32, limit: u32) -> Vec<u64> {
    let count_key = DataKey::BySenderCount(sender.clone());
    let legacy_key = DataKey::BySender(sender.clone());
    read_index(env, &count_key, &legacy_key, |page| DataKey::BySenderPage(sender.clone(), page), offset, limit)
}

pub fn streams_by_recipient(env: &Env, recipient: Address, offset: u32, limit: u32) -> Vec<u64> {
    let count_key = DataKey::ByRecipientCount(recipient.clone());
    let legacy_key = DataKey::ByRecipient(recipient.clone());
    read_index(env, &count_key, &legacy_key, |page| {
        DataKey::ByRecipientPage(recipient.clone(), page)
    }, offset, limit)
}

pub fn stream_count_by_sender(env: &Env, sender: Address) -> u32 {
    let count_key = DataKey::BySenderCount(sender.clone());
    let legacy_key = DataKey::BySender(sender);
    count_index(env, &count_key, &legacy_key)
}

pub fn stream_count_by_recipient(env: &Env, recipient: Address) -> u32 {
    let count_key = DataKey::ByRecipientCount(recipient.clone());
    let legacy_key = DataKey::ByRecipient(recipient);
    count_index(env, &count_key, &legacy_key)
}
