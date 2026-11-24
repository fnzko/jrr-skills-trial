# How the Rust rules apply when you change existing code

The code already here may not follow the rules below. That is not a reason to break them in what you add.

- Every line you add follows the rules, even where the code next to it does not.
- When existing code forces new code to break a rule, change the existing code just enough for the new code to follow it. Small changes like that are part of the task.
- Do not go further: no rewrite of functions the change does not need to touch, and no migration of existing code to the rules.
- Rules that only hold crate-wide apply to the data this change introduces. Do not retype existing data.

