let s = 'month';
let ss = 's';

select 1 as f"foo_{s}";
select f"{ss}" from (select 1 as "s"); -- should be the value 1

-- TODO
-- select f'{ss}'; -- should be the value s

let slices = ['month', 'day'];

select
    for item in slices {
        item AS f"metric_{item}"
    }
;
